//! A window's UI Automation tree: the control view, depth first. A windowless parent's children come back in one
//! cross-process call (`TreeScope_Children` + `BuildUpdatedCache`) with every property Cay reads, the runtime id
//! included. A parent with its own HWND is walked a sibling at a time instead: the one-call fetch leaves out child
//! windows another provider adds (a XAML island under a WebView2 control did; D59, C16). Bounded three ways: depth,
//! node count, and a wall-clock budget (plus UIA's own
//! connection and transaction timeouts for hung providers). Needs an MTA thread (`mod.rs` runs it on one).

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, SAFEARRAY};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{VariantClear, VARENUM, VT_ARRAY, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationTreeWalker, TreeScope, TreeScope_Children, TreeScope_Element,
    UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ControlTypePropertyId,
    UIA_HasKeyboardFocusPropertyId, UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_IsOffscreenPropertyId, UIA_IsPasswordPropertyId, UIA_NamePropertyId,
    UIA_NativeWindowHandlePropertyId, UIA_RuntimeIdPropertyId, UIA_ValueValuePropertyId,
};

use super::logic::Rect;
use super::tree::{Caps, Cut, Node};
use crate::win32;

/// The whole walk gets this long; a big browser page stops early and says so.
const BUDGET: Duration = Duration::from_secs(3);
const UIA_TIMEOUT_MS: u32 = 2000;
/// Most children kept per parent (a huge list is cut by `caps.nodes` long before this).
const MAX_CHILDREN: i32 = 2000;

/// UI Automation with Cay's timeouts, so a hung provider can't hold a thread for long. Needs COM (MTA).
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

/// What a walk found: the nodes, why it stopped early, and how many offscreen elements it left out (without
/// `caps.offscreen`, an offscreen element is skipped with everything inside it).
pub struct Walked {
    pub nodes: Vec<Node>,
    pub cut: Option<Cut>,
    pub offscreen: u32,
}

/// Walks window `hwnd`, or only `from` (an element of it) and what's inside.
pub fn walk(
    hwnd: isize,
    from: Option<&IUIAutomationElement>,
    caps: &Caps,
) -> windows::core::Result<Walked> {
    let started = Instant::now();
    let uia = automation()?;
    // SAFETY: COM calls on smart pointers released on drop; the caller initialised COM (MTA) on this thread.
    unsafe {
        let element_cache = uia.CreateCacheRequest()?;
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
            UIA_RuntimeIdPropertyId,
            UIA_NativeWindowHandlePropertyId,
        ] {
            cache.AddProperty(id)?;
            element_cache.AddProperty(id)?;
        }
        element_cache.SetTreeScope(TreeScope_Element)?;
        // The element and its children (control view: the cache request's default filter), properties for all.
        cache.SetTreeScope(TreeScope(TreeScope_Element.0 | TreeScope_Children.0))?;
        let fetch = Fetch {
            children: cache,
            element: element_cache,
            walker: uia.ControlViewWalker()?,
        };
        let root = match from {
            Some(e) => e.BuildUpdatedCache(&fetch.element)?,
            None => uia.ElementFromHandleBuildCache(win32::from_raw(hwnd), &fetch.element)?,
        };

        let mut nodes = Vec::new();
        let mut cut = None;
        let mut offscreen = 0;
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
            let n = node(&element, depth);
            // The window (or the root asked for) is always kept, even when UIA calls it offscreen.
            if depth > 0 && n.offscreen && !caps.offscreen {
                offscreen += 1;
                continue;
            }
            nodes.push(n);
            if depth + 1 >= caps.depth {
                continue;
            }
            // Children are pushed in reverse so the first child is visited next.
            let children = fetch.children_of(&element, started);
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        Ok(Walked {
            nodes,
            cut,
            offscreen,
        })
    }
}

/// How children are fetched: in one call for a windowless parent, through the walker for one with an HWND.
struct Fetch {
    /// Element and children, every property.
    children: IUIAutomationCacheRequest,
    /// The element alone, every property (for the walker).
    element: IUIAutomationCacheRequest,
    walker: IUIAutomationTreeWalker,
}

impl Fetch {
    /// A parent's children; a failed call is an element without children.
    unsafe fn children_of(
        &self,
        parent: &IUIAutomationElement,
        started: Instant,
    ) -> Vec<IUIAutomationElement> {
        // SAFETY: COM calls on live smart pointers; the array is read within its length.
        unsafe {
            let own_window = parent
                .CachedNativeWindowHandle()
                .is_ok_and(|h| !h.is_invalid());
            if own_window {
                let mut out = Vec::new();
                let mut next = self
                    .walker
                    .GetFirstChildElementBuildCache(parent, &self.element)
                    .ok();
                while let Some(child) = next {
                    if started.elapsed() > BUDGET || out.len() >= MAX_CHILDREN as usize {
                        break;
                    }
                    next = self
                        .walker
                        .GetNextSiblingElementBuildCache(&child, &self.element)
                        .ok();
                    out.push(child);
                }
                return out;
            }
            let Ok(parent) = parent.BuildUpdatedCache(&self.children) else {
                return Vec::new();
            };
            let Ok(array) = parent.GetCachedChildren() else {
                return Vec::new();
            };
            let len = array.Length().unwrap_or(0).clamp(0, MAX_CHILDREN);
            (0..len).filter_map(|i| array.GetElement(i).ok()).collect()
        }
    }
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
            runtime_id: cached_runtime_id(e).unwrap_or_else(|| runtime_id(e)),
        }
    }
}

/// The runtime id from the cache (a VT_ARRAY of VT_I4), or `None` to ask the element directly.
unsafe fn cached_runtime_id(e: &IUIAutomationElement) -> Option<Vec<i32>> {
    // SAFETY: the VARIANT's tag is checked before reading the array arm; VariantClear frees the array.
    unsafe {
        let mut v = e.GetCachedPropertyValue(UIA_RuntimeIdPropertyId).ok()?;
        let inner = &v.Anonymous.Anonymous;
        let ids =
            (inner.vt == VARENUM(VT_ARRAY.0 | VT_I4.0)).then(|| read_i32s(inner.Anonymous.parray));
        let _ = VariantClear(&mut v);
        ids.filter(|ids| !ids.is_empty())
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
