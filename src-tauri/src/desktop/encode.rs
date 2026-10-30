//! Capture → image bytes with WIC (part of Windows, so no new crate): scaled with the Fant filter so the long edge
//! is at most `MAX_EDGE`, then PNG, or JPEG when PNG is over `MAX_IMAGE_BYTES` (photos, video). Needs COM on the
//! calling thread.

use windows::core::{GUID, PWSTR};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGR, IWICBitmapFrameEncode,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
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
    let (width, height) = fit(bitmap.width, bitmap.height, MAX_EDGE);
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
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&source, width, height, WICBitmapInterpolationModeFant)?;
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
            let bytes = write(&factory, &converter, width, height, encoding)?;
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
