//! Capture → image bytes with WIC (part of Windows, so no new crate): scaled with the Fant filter so the long edge
//! is at most `MAX_EDGE`, then PNG, or JPEG when PNG is over `MAX_IMAGE_BYTES` (photos, video). Needs COM on the
//! calling thread.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGR, IWICBitmapFrameEncode,
    IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{
    CoCreateInstance, IStream, CLSCTX_INPROC_SERVER, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::System::Variant::{VARIANT, VT_R4};
use windows::Win32::UI::Shell::SHCreateMemStream;

use super::capture::Bitmap;
use super::logic::{fit, Encoding, ENCODINGS, MAX_EDGE, MAX_IMAGE_BYTES};

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

pub fn encode(bitmap: &Bitmap) -> windows::core::Result<Encoded> {
    encode_within(bitmap, MAX_EDGE)
}

/// `encode` with a smaller long edge (a screenshot's `maxEdge`; at most `MAX_EDGE`).
pub fn encode_within(bitmap: &Bitmap, max_edge: u32) -> windows::core::Result<Encoded> {
    // SAFETY: plain WIC calls on COM smart pointers; `bitmap.pixels` holds width*height*4 bytes and WIC copies
    // them in CreateBitmapFromMemory.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let source = factory.CreateBitmapFromMemory(
            bitmap.width,
            bitmap.height,
            &GUID_WICPixelFormat32bppBGR,
            bitmap.width * 4,
            &bitmap.pixels,
        )?;
        encode_source(
            &factory,
            &source.cast()?,
            bitmap.width,
            bitmap.height,
            max_edge.min(MAX_EDGE),
        )
    }
}

/// An image file (an attachment, 2.10b) scaled and encoded the same way: the first frame only, and no metadata
/// (EXIF, GPS) survives. Transparent areas come out black, as in a capture.
pub fn encode_file(path: &Path) -> windows::core::Result<Encoded> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: plain WIC calls on COM smart pointers; `wide` is NUL-terminated and outlives the call.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(wide.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0u32, 0u32);
        frame.GetSize(&mut width, &mut height)?;
        if width == 0 || height == 0 {
            return Err(windows::core::Error::empty());
        }
        encode_source(&factory, &frame.cast()?, width, height, MAX_EDGE)
    }
}

/// Scales `source` so its long edge is at most `max_edge`, then encodes it (PNG, else JPEG) to fit.
unsafe fn encode_source(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    source_width: u32,
    source_height: u32,
    max_edge: u32,
) -> windows::core::Result<Encoded> {
    let (width, height) = fit(source_width, source_height, max_edge);
    // SAFETY: the caller's COM objects; every object made here is released on drop.
    unsafe {
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(source, width, height, WICBitmapInterpolationModeFant)?;
        // The encoders take 24-bit BGR (the capture's fourth byte is padding, not alpha).
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat24bppBGR,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut last = None;
        for &encoding in ENCODINGS {
            let bytes = write(factory, &converter, width, height, encoding)?;
            let fits = bytes.len() <= MAX_IMAGE_BYTES;
            last = Some(Encoded {
                bytes,
                mime: encoding.mime(),
                width,
                height,
            });
            if fits {
                break;
            }
        }
        last.ok_or_else(windows::core::Error::empty)
    }
}

unsafe fn write(
    factory: &IWICImagingFactory,
    source: &windows::Win32::Graphics::Imaging::IWICFormatConverter,
    width: u32,
    height: u32,
    encoding: Encoding,
) -> windows::core::Result<Vec<u8>> {
    // SAFETY: the caller's COM objects; the stream and encoder are released on drop; the property bag's name
    // buffer outlives the Write call.
    unsafe {
        let stream: IStream = SHCreateMemStream(None).ok_or_else(windows::core::Error::empty)?;
        let container: GUID = match encoding {
            Encoding::Png => GUID_ContainerFormatPng,
            Encoding::Jpeg(_) => GUID_ContainerFormatJpeg,
        };
        let encoder = factory.CreateEncoder(&container, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut options: Option<IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        if let (Encoding::Jpeg(quality), Some(options)) = (encoding, &options) {
            let mut name: Vec<u16> = "ImageQuality\0".encode_utf16().collect();
            let bag = PROPBAG2 {
                pstrName: PWSTR(name.as_mut_ptr()),
                ..Default::default()
            };
            let mut value = VARIANT::default();
            (*value.Anonymous.Anonymous).vt = VT_R4;
            (*value.Anonymous.Anonymous).Anonymous.fltVal = quality;
            options.Write(1, &bag, &value)?;
        }
        frame.Initialize(options.as_ref())?;
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;

        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let len = stat.cbSize as usize;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = vec![0u8; len];
        let mut read = 0u32;
        stream
            .Read(bytes.as_mut_ptr().cast(), len as u32, Some(&mut read))
            .ok()?;
        bytes.truncate(read as usize);
        Ok(bytes)
    }
}
