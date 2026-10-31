//! UI Automation actions (Phase 2.7d): find an element from `ui_tree` again, check it's still the same element,
//! and use its pattern (Invoke, Toggle, SelectionItem, ExpandCollapse, ScrollItem, Value, focus). Never acts on
//! a guess: the runtime id must match, or a unique element with the same automation id, type and name. Needs an
//! MTA thread (`mod.rs` runs actions on one).

use windows::core::{Interface, BSTR, HRESULT};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    ExpandCollapseState_Collapsed, IUIAutomation, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationScrollItemPattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern,
    TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_ControlTypePropertyId,
    UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_LegacyIAccessiblePatternId,
    UIA_ScrollItemPatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
    UIA_CONTROLTYPE_ID, UIA_PATTERN_ID,
};

use super::logic::Rect;
use super::plan::{ActError, Element, Verb};
use super::tree::control_type_name;
use super::uia;
use crate::win32;

// UIA's error codes (UIAutomationCoreApi.h).
const UIA_E_ELEMENTNOTENABLED: HRESULT = HRESULT(0x8004_0200_u32 as i32);
const UIA_E_ELEMENTNOTAVAILABLE: HRESULT = HRESULT(0x8004_0201_u32 as i32);
const UIA_E_INVALIDOPERATION: HRESULT = HRESULT(0x8013_1509_u32 as i32);
const UIA_E_TIMEOUT: HRESULT = HRESULT(0x8013_1505_u32 as i32);
/// How many parents of the element at the old position are compared before searching the tree.
const POINT_PARENTS: usize = 6;

/// What a UIA action did, for the reply.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// "Invoked button "Save"".
    pub what: String,
    /// The app didn't answer in time; the action may still have happened (e.g. it opened a dialog).
    pub unconfirmed: bool,
    /// After `set_value`: whether the field now holds the value.
    pub value_matches: Option<bool>,
}

fn err(e: windows::core::Error) -> ActError {
    match e.code() {
        UIA_E_ELEMENTNOTAVAILABLE => ActError::ElementGone,
        UIA_E_ELEMENTNOTENABLED => ActError::Disabled,
        UIA_E_INVALIDOPERATION => ActError::Unsupported("the app refused that operation".into()),
        _ => ActError::Failed(e.message().to_string()),
    }
}

/// `what` was sent; a timeout means "probably happened, not confirmed".
fn sent(result: windows::core::Result<()>, what: String) -> Result<Done, ActError> {
    match result {
        Ok(()) => Ok(Done {
            what,
            unconfirmed: false,
            value_matches: None,
        }),
        Err(e) if e.code() == UIA_E_TIMEOUT => Ok(Done {
            what,
            unconfirmed: true,
            value_matches: None,
        }),
        Err(e) => Err(err(e)),
    }
}

fn label(e: &Element) -> String {
    let kind = control_type_name(e.control_type);
    if e.name.trim().is_empty() {
        kind.to_string()
    } else {
        format!("{kind} \"{}\"", e.name.trim())
    }
}

/// Finds `wanted` in window `hwnd` (process `pid`) again and checks it's enabled.
pub fn find(
    uia: &IUIAutomation,
    hwnd: isize,
    pid: u32,
    wanted: &Element,
) -> Result<IUIAutomationElement, ActError> {
    let found = by_point(uia, pid, wanted).or_else(|| by_search(uia, hwnd, wanted)?.ok());
    let found = match found {
        Some(e) => e,
        None => {
            return Err(by_search(uia, hwnd, wanted)
                .and_then(Result::err)
                .unwrap_or(ActError::ElementGone))
        }
    };
    // SAFETY: property reads on a live element.
    unsafe {
        if found.CurrentProcessId().is_ok_and(|p| p as u32 != pid) {
            return Err(ActError::ElementGone);
        }
        let same = found
            .CurrentControlType()
            .is_ok_and(|t| t.0 == wanted.control_type)
            && found
                .CurrentAutomationId()
                .is_ok_and(|a| a == wanted.automation_id)
            && found
                .CurrentName()
                .is_ok_and(|n| n.to_string().trim() == wanted.name.trim());
        if !same {
            return Err(ActError::ElementGone);
        }
        if !found.CurrentIsEnabled().is_ok_and(|b| b.as_bool()) {
            return Err(ActError::Disabled);
        }
    }
    Ok(found)
}

/// The element at the old position, or one of its parents, with the same runtime id.
fn by_point(uia: &IUIAutomation, pid: u32, wanted: &Element) -> Option<IUIAutomationElement> {
    if wanted.rect.is_empty() || wanted.runtime_id.is_empty() {
        return None;
    }
    let center = POINT {
        x: wanted.rect.x + wanted.rect.width / 2,
        y: wanted.rect.y + wanted.rect.height / 2,
    };
    // SAFETY: COM calls on live smart pointers.
    unsafe {
        let walker = uia.ControlViewWalker().ok()?;
        let mut e = uia.ElementFromPoint(center).ok()?;
        for _ in 0..POINT_PARENTS {
            if e.CurrentProcessId().is_ok_and(|p| p as u32 != pid) {
                return None;
            }
            if uia::runtime_id(&e) == wanted.runtime_id {
                return Some(e);
            }
            e = walker.GetParentElement(&e).ok()?;
        }
    }
    None
}

/// Elements of the window with the same type and automation id (and name, when there's no automation id): the
/// one with the same runtime id, or the only one when the automation id identifies it. `Some(Err)`: ambiguous.
fn by_search(
    uia: &IUIAutomation,
    hwnd: isize,
    wanted: &Element,
) -> Option<Result<IUIAutomationElement, ActError>> {
    // SAFETY: COM calls on live smart pointers; VARIANTs are dropped (cleared) by their wrappers.
    unsafe {
        let root = uia.ElementFromHandle(win32::from_raw(hwnd)).ok()?;
        let by_type = uia
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(wanted.control_type),
            )
            .ok()?;
        let key = if wanted.automation_id.is_empty() {
            (
                windows::Win32::UI::Accessibility::UIA_NamePropertyId,
                wanted.name.as_str(),
            )
        } else {
            (UIA_AutomationIdPropertyId, wanted.automation_id.as_str())
        };
        let by_key = uia
            .CreatePropertyCondition(key.0, &VARIANT::from(BSTR::from(key.1)))
            .ok()?;
        let both = uia.CreateAndCondition(&by_type, &by_key).ok()?;
        let all = root.FindAll(TreeScope_Descendants, &both).ok()?;
        let n = all.Length().ok()?;
        let mut candidates = Vec::new();
        for i in 0..n.min(50) {
            if let Ok(e) = all.GetElement(i) {
                if uia::runtime_id(&e) == wanted.runtime_id && !wanted.runtime_id.is_empty() {
                    return Some(Ok(e));
                }
                candidates.push(e);
            }
        }
        match candidates.len() {
            1 if !wanted.automation_id.is_empty() => candidates.pop().map(Ok),
            0 | 1 => None,
            _ => Some(Err(ActError::ElementGone)),
        }
    }
}

fn pattern<T: Interface>(e: &IUIAutomationElement, id: UIA_PATTERN_ID) -> Option<T> {
    // SAFETY: asks the element for a pattern interface; `None` if it doesn't support it.
    unsafe { e.GetCurrentPatternAs::<T>(id).ok() }
}

/// Uses `verb` on the element (for `Default`: its main pattern).
pub fn invoke(e: &IUIAutomationElement, wanted: &Element, verb: Verb) -> Result<Done, ActError> {
    let name = label(wanted);
    // SAFETY: pattern calls on live smart pointers.
    unsafe {
        match verb {
            Verb::Invoke => match pattern::<IUIAutomationInvokePattern>(e, UIA_InvokePatternId) {
                Some(p) => sent(p.Invoke(), format!("Invoked {name}")),
                None => Err(ActError::Unsupported(format!("{name} can't be invoked"))),
            },
            Verb::Toggle => match pattern::<IUIAutomationTogglePattern>(e, UIA_TogglePatternId) {
                Some(p) => sent(p.Toggle(), format!("Toggled {name}")),
                None => Err(ActError::Unsupported(format!("{name} can't be toggled"))),
            },
            Verb::Select => {
                match pattern::<IUIAutomationSelectionItemPattern>(e, UIA_SelectionItemPatternId) {
                    Some(p) => sent(p.Select(), format!("Selected {name}")),
                    None => Err(ActError::Unsupported(format!("{name} can't be selected"))),
                }
            }
            Verb::Expand | Verb::Collapse => {
                match pattern::<IUIAutomationExpandCollapsePattern>(e, UIA_ExpandCollapsePatternId)
                {
                    Some(p) if verb == Verb::Expand => sent(p.Expand(), format!("Expanded {name}")),
                    Some(p) => sent(p.Collapse(), format!("Collapsed {name}")),
                    None => Err(ActError::Unsupported(format!(
                        "{name} can't be expanded or collapsed"
                    ))),
                }
            }
            Verb::ScrollIntoView => {
                match pattern::<IUIAutomationScrollItemPattern>(e, UIA_ScrollItemPatternId) {
                    Some(p) => sent(p.ScrollIntoView(), format!("Scrolled {name} into view")),
                    None => Err(ActError::Unsupported(format!(
                        "{name} can't be scrolled into view"
                    ))),
                }
            }
            Verb::Focus => sent(e.SetFocus(), format!("Focused {name}")),
            Verb::Default => {
                if let Some(p) = pattern::<IUIAutomationInvokePattern>(e, UIA_InvokePatternId) {
                    return sent(p.Invoke(), format!("Invoked {name}"));
                }
                if let Some(p) = pattern::<IUIAutomationTogglePattern>(e, UIA_TogglePatternId) {
                    return sent(p.Toggle(), format!("Toggled {name}"));
                }
                if let Some(p) =
                    pattern::<IUIAutomationSelectionItemPattern>(e, UIA_SelectionItemPatternId)
                {
                    return sent(p.Select(), format!("Selected {name}"));
                }
                if let Some(p) =
                    pattern::<IUIAutomationExpandCollapsePattern>(e, UIA_ExpandCollapsePatternId)
                {
                    let collapsed = p
                        .CurrentExpandCollapseState()
                        .is_ok_and(|s| s == ExpandCollapseState_Collapsed);
                    return if collapsed {
                        sent(p.Expand(), format!("Expanded {name}"))
                    } else {
                        sent(p.Collapse(), format!("Collapsed {name}"))
                    };
                }
                if let Some(p) = pattern::<IUIAutomationLegacyIAccessiblePattern>(
                    e,
                    UIA_LegacyIAccessiblePatternId,
                ) {
                    if p.CurrentDefaultAction().is_ok_and(|a| !a.is_empty()) {
                        return sent(
                            p.DoDefaultAction(),
                            format!("Did the default action of {name}"),
                        );
                    }
                }
                Err(ActError::Unsupported(format!(
                    "{name} has no invoke, toggle, select or expand pattern; try click with its ref"
                )))
            }
        }
    }
}

/// Sets a field's text through the Value pattern, then reads it back.
pub fn set_value(
    e: &IUIAutomationElement,
    wanted: &Element,
    value: &str,
) -> Result<Done, ActError> {
    let name = label(wanted);
    // SAFETY: property reads and pattern calls on live smart pointers.
    unsafe {
        if e.CurrentIsPassword().is_ok_and(|b| b.as_bool()) {
            return Err(ActError::PasswordField);
        }
        let Some(p) = pattern::<IUIAutomationValuePattern>(e, UIA_ValuePatternId) else {
            return Err(ActError::Unsupported(format!(
                "{name} has no value to set; focus it and use type_text"
            )));
        };
        if p.CurrentIsReadOnly().is_ok_and(|b| b.as_bool()) {
            return Err(ActError::Unsupported(format!("{name} is read-only")));
        }
        let mut done = sent(p.SetValue(&BSTR::from(value)), format!("Set {name}"))?;
        let now = p.CurrentValue().map(|v| v.to_string());
        done.value_matches = now
            .ok()
            .map(|v| v.replace("\r\n", "\n") == value.replace("\r\n", "\n"));
        Ok(done)
    }
}

/// The element's current centre, for a click on it. Refused if it's off screen.
pub fn center(e: &IUIAutomationElement) -> Result<(i32, i32), ActError> {
    // SAFETY: property reads on a live element.
    unsafe {
        if e.CurrentIsOffscreen().is_ok_and(|b| b.as_bool()) {
            return Err(ActError::Covered);
        }
        let r = e.CurrentBoundingRectangle().map_err(err)?;
        let rect = Rect::from_ltrb(r.left, r.top, r.right, r.bottom);
        if rect.is_empty() {
            return Err(ActError::Covered);
        }
        Ok((rect.x + rect.width / 2, rect.y + rect.height / 2))
    }
}

/// The element with keyboard focus: whether it's a password field, and its label ("edit "Search"").
pub fn focused(uia: &IUIAutomation) -> Option<(bool, String)> {
    // SAFETY: COM calls on live smart pointers.
    unsafe {
        let e = uia.GetFocusedElement().ok()?;
        let password = e.CurrentIsPassword().is_ok_and(|b| b.as_bool());
        let kind = e.CurrentControlType().unwrap_or(UIA_CONTROLTYPE_ID(0));
        let name = e.CurrentName().map(|n| n.to_string()).unwrap_or_default();
        let el = Element {
            control_type: kind.0,
            name,
            ..Element::default()
        };
        Some((password, label(&el)))
    }
}
