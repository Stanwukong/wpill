//! A window's UI Automation tree: the control view, depth first, one cache request per element so each step is a
//! single cross-process call. Bounded three ways: depth, node count, and a wall-clock budget (plus UIA's own
//! connection and transaction timeouts for hung providers). Needs an MTA thread (`mod.rs` runs it on one).

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, SAFEARRAY};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{VariantClear, VT_BSTR};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationTreeWalker, TreeScope_Element, UIA_AutomationIdPropertyId,
    UIA_BoundingRectanglePropertyId, UIA_ControlTypePropertyId, UIA_HasKeyboardFocusPropertyId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId,
    UIA_IsPasswordPropertyId, UIA_NamePropertyId, UIA_ValueValuePropertyId,
};

use super::logic::Rect;
use super::tree::{Caps, Cut, Node};
use crate::win32;

/// The whole walk gets this long; a big browser page stops early and says so.
const BUDGET: Duration = Duration::from_secs(3);
const UIA_TIMEOUT_MS: u32 = 2000;

/// UI Automation with TopBar's timeouts, so a hung provider can't hold a thread for long. Needs COM (MTA).
pub fn automation() -> windows::core::Result<IUIAutomation> {
    // SAFETY: COM calls on smart pointers released on drop; the caller initialised COM (MTA) on this thread.
    unsafe {
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
            let _ = uia2.SetConnectionTimeout(UIA_TIMEOUT_MS);
            let _ = uia2.SetTransactionTimeout(UIA_TIMEOUT_MS);
        }
        Ok(uia)
    }
}

/// An element's UIA runtime id (empty if it has none).
pub fn runtime_id(e: &IUIAutomationElement) -> Vec<i32> {
    // SAFETY: GetRuntimeId returns a SAFEARRAY of VT_I4 that we own; it's read within its bounds, then destroyed.
    unsafe {
        let Ok(array) = e.GetRuntimeId() else {
            return Vec::new();
        };
        let ids = read_i32s(array);
        let _ = SafeArrayDestroy(array);
        ids
    }
}

unsafe fn read_i32s(array: *const SAFEARRAY) -> Vec<i32> {
    if array.is_null() {
        return Vec::new();
    }
    // SAFETY: a one-dimensional VT_I4 array (what UIA returns); the data pointer is valid between access and
    // unaccess, for `len` elements.
    unsafe {
        let (Ok(lo), Ok(hi)) = (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) else {
            return Vec::new();
        };
        let len = (hi - lo + 1).max(0) as usize;
        let mut data = std::ptr::null_mut();
        if len == 0 || SafeArrayAccessData(array, &mut data).is_err() {
            return Vec::new();
        }
        let ids = std::slice::from_raw_parts(data as *const i32, len).to_vec();
        let _ = SafeArrayUnaccessData(array);
        ids
    }
}

pub fn walk(hwnd: isize, caps: &Caps) -> windows::core::Result<(Vec<Node>, Option<Cut>)> {
    let started = Instant::now();
    let uia = automation()?;
    // SAFETY: COM calls on smart pointers released on drop; the caller initialised COM (MTA) on this thread.
    unsafe {
        let cache = uia.CreateCacheRequest()?;
        for id in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_AutomationIdPropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_IsKeyboardFocusablePropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_IsOffscreenPropertyId,
            UIA_IsPasswordPropertyId,
            UIA_ValueValuePropertyId,
        ] {
            cache.AddProperty(id)?;
        }
        cache.SetTreeScope(TreeScope_Element)?;
        let walker = uia.ControlViewWalker()?;
        let root = uia.ElementFromHandleBuildCache(win32::from_raw(hwnd), &cache)?;

        let mut nodes = Vec::new();
        let mut cut = None;
        // Depth-first with an explicit stack of (element, depth).
        let mut stack: Vec<(IUIAutomationElement, u32)> = vec![(root, 0)];
        while let Some((element, depth)) = stack.pop() {
            if nodes.len() >= caps.nodes as usize {
                cut = Some(Cut::Nodes);
                break;
            }
            if started.elapsed() > BUDGET {
                cut = Some(Cut::Time);
                break;
            }
            nodes.push(node(&element, depth));
            if depth + 1 >= caps.depth {
                continue;
            }
            // Children are pushed in reverse so the first child is visited next.
            let children = children(&walker, &cache, &element, started);
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        Ok((nodes, cut))
    }
}

unsafe fn children(
    walker: &IUIAutomationTreeWalker,
    cache: &IUIAutomationCacheRequest,
    parent: &IUIAutomationElement,
    started: Instant,
) -> Vec<IUIAutomationElement> {
    let mut out = Vec::new();
    // SAFETY: COM calls on live smart pointers; a failed call ends the list.
    unsafe {
        let mut next = walker.GetFirstChildElementBuildCache(parent, cache).ok();
        while let Some(child) = next {
            if started.elapsed() > BUDGET || out.len() >= 2000 {
                break;
            }
            next = walker.GetNextSiblingElementBuildCache(&child, cache).ok();
            out.push(child);
        }
    }
    out
}

unsafe fn node(e: &IUIAutomationElement, depth: u32) -> Node {
    // SAFETY: cached property reads on a live element; each failure falls back to a default.
    unsafe {
        let password = e.CachedIsPassword().is_ok_and(|b| b.as_bool());
        let r = e.CachedBoundingRectangle().unwrap_or_default();
        Node {
            depth,
            control_type: e.CachedControlType().map_or(0, |t| t.0),
            name: e.CachedName().map(|s| s.to_string()).unwrap_or_default(),
            // Never read a password field's value at all.
            value: if password { None } else { value(e) },
            automation_id: e
                .CachedAutomationId()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            rect: Rect::from_ltrb(r.left, r.top, r.right, r.bottom),
            enabled: e.CachedIsEnabled().is_ok_and(|b| b.as_bool()),
            focusable: e.CachedIsKeyboardFocusable().is_ok_and(|b| b.as_bool()),
            focused: e.CachedHasKeyboardFocus().is_ok_and(|b| b.as_bool()),
            offscreen: e.CachedIsOffscreen().is_ok_and(|b| b.as_bool()),
            password,
            runtime_id: runtime_id(e),
        }
    }
}

/// The Value pattern's text, when the element has one (a BSTR; anything else is "not supported").
unsafe fn value(e: &IUIAutomationElement) -> Option<String> {
    // SAFETY: the VARIANT's tag is checked before reading the BSTR arm; VariantClear frees it.
    unsafe {
        let mut v = e.GetCachedPropertyValue(UIA_ValueValuePropertyId).ok()?;
        let inner = &v.Anonymous.Anonymous;
        let text = (inner.vt == VT_BSTR).then(|| inner.Anonymous.bstrVal.to_string());
        let _ = VariantClear(&mut v);
        text.filter(|t| !t.is_empty())
    }
}
