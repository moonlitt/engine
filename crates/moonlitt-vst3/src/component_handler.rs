//! IComponentHandler implementation.
//!
//! Plugins (specifically their IEditController side) call into this to notify
//! the host when a parameter changes — typically `performEdit(paramId, value)`
//! after the user moves a knob, or programmatic state changes triggered by
//! `setParamNormalized`. The host's job is to forward these into the next
//! audio block via `ProcessData::inputParameterChanges` so the processor
//! actually hears about them.
//!
//! Without this hookup, all controller-side parameter writes are silently
//! dropped — sampler-style plugins (Keyscape, Kontakt, sfizz) never receive
//! their patch selection and stay silent.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandler2, IComponentHandler2Trait, IComponentHandler3,
    IComponentHandler3Trait, IComponentHandlerTrait, IContextMenu, IUnitHandler, IUnitHandler2,
    IUnitHandler2Trait, IUnitHandlerTrait, ParamID, ParamValue, ProgramListID, UnitID,
};
use vst3::Steinberg::{int32, kResultOk, tresult, FIDString, IPlugView, TBool};
use vst3::{Class, ComWrapper};

/// One pending controller→processor parameter change.
///
/// `sample_offset` lets the host position the edit at a sub-block sample
/// — required for sample-accurate automation from a DAW timeline. The
/// VST3 spec accepts any value in `[0, num_samples)`; the plug-in
/// interpolates between adjacent points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingParam {
    pub id: ParamID,
    pub value: ParamValue,
    pub sample_offset: i32,
}

/// Shared queue between the COM impl (writer, called by the plugin's
/// controller from any thread) and the audio render path (reader/drainer).
pub(crate) type ParamQueue = Arc<Mutex<Vec<PendingParam>>>;

/// OR-accumulator for `IComponentHandler::restartComponent` flag bits.
/// The plug-in writes from arbitrary threads; the render path drains on
/// each render and dispatches handling.
pub(crate) type RestartFlags = Arc<AtomicI32>;

/// Side-band notifications the plug-in pushes through IComponentHandler2 /
/// IUnitHandler / IUnitHandler2. These don't go through the parameter
/// pipeline — they describe host-side concerns (project dirty bit, editor
/// open requests, program/unit changes the host should reflect in UI).
#[derive(Clone, Debug, PartialEq)]
pub enum HostNotification {
    /// Plug-in reports its persistent state is dirty (or clean).
    SetDirty(bool),
    /// Plug-in asked the host to open an editor with the given name. Empty
    /// string is allowed (means "the default editor").
    RequestOpenEditor(String),
    /// Begin a group of edits — host should treat subsequent performEdit
    /// calls as one atomic undo step until [`FinishGroupEdit`].
    StartGroupEdit,
    /// End of a group edit started by [`StartGroupEdit`].
    FinishGroupEdit,
    /// Selected unit changed inside the plug-in (multi-timbral samplers).
    UnitSelection(UnitID),
    /// One of the plug-in's program lists changed; host should refresh its
    /// preset cache for the named list.
    ProgramListChange {
        list_id: ProgramListID,
        program_index: int32,
    },
    /// The plug-in's unit/bus mapping changed; host should re-query the
    /// IUnitInfo topology.
    UnitByBusChange,
}

/// Shared queue between the COM impls (any thread, called by the plug-in)
/// and consumers polling host-side state (typically the audio thread or a
/// UI poll on the control thread).
pub(crate) type NotificationQueue = Arc<Mutex<Vec<HostNotification>>>;

pub(crate) struct ComponentHandler {
    queue: ParamQueue,
    restart_flags: RestartFlags,
    notifications: NotificationQueue,
}

impl ComponentHandler {
    fn push_notification(&self, n: HostNotification) {
        if let Ok(mut q) = self.notifications.lock() {
            q.push(n);
        }
    }
}

impl Class for ComponentHandler {
    type Interfaces = (
        IComponentHandler,
        IComponentHandler2,
        IComponentHandler3,
        IUnitHandler,
        IUnitHandler2,
    );
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        crate::trace::emit(&format!("ComponentHandler::beginEdit id={id}"));
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        crate::trace::emit(&format!(
            "ComponentHandler::performEdit id={id} value={value_normalized}"
        ));
        if let Ok(mut q) = self.queue.lock() {
            q.push(PendingParam {
                id,
                value: value_normalized,
                sample_offset: 0,
            });
        }
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        crate::trace::emit(&format!("ComponentHandler::endEdit id={id}"));
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        crate::trace::emit(&format!(
            "ComponentHandler::restartComponent flags=0x{flags:08X}"
        ));
        // OR the new flag bits into the shared accumulator. The render
        // path reads-and-clears this on each block and dispatches to
        // whichever flag-specific handler applies.
        self.restart_flags.fetch_or(flags, Ordering::AcqRel);
        kResultOk
    }
}

impl IComponentHandler2Trait for ComponentHandler {
    unsafe fn setDirty(&self, state: TBool) -> tresult {
        let dirty = state != 0;
        crate::trace::emit(&format!("ComponentHandler2::setDirty state={dirty}"));
        self.push_notification(HostNotification::SetDirty(dirty));
        kResultOk
    }

    unsafe fn requestOpenEditor(&self, name: FIDString) -> tresult {
        let name_str = if name.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(name)
                .to_string_lossy()
                .into_owned()
        };
        crate::trace::emit(&format!(
            "ComponentHandler2::requestOpenEditor name=\"{name_str}\""
        ));
        self.push_notification(HostNotification::RequestOpenEditor(name_str));
        kResultOk
    }

    unsafe fn startGroupEdit(&self) -> tresult {
        crate::trace::emit("ComponentHandler2::startGroupEdit");
        self.push_notification(HostNotification::StartGroupEdit);
        kResultOk
    }

    unsafe fn finishGroupEdit(&self) -> tresult {
        crate::trace::emit("ComponentHandler2::finishGroupEdit");
        self.push_notification(HostNotification::FinishGroupEdit);
        kResultOk
    }
}

impl IComponentHandler3Trait for ComponentHandler {
    unsafe fn createContextMenu(
        &self,
        _plug_view: *mut IPlugView,
        param_id: *const ParamID,
    ) -> *mut IContextMenu {
        // VST3 spec: returning null is acceptable — the plug-in falls
        // back to its own built-in context menu. Our headless host
        // doesn't have an automation/MIDI-learn UI to inject menu items
        // into, so null is the correct answer. Tracing helps confirm
        // the plug-in asked (some plug-ins skip entirely when null is
        // already known via interface enumeration).
        let pid = if param_id.is_null() {
            0
        } else {
            *param_id
        };
        crate::trace::emit(&format!(
            "ComponentHandler3::createContextMenu param={pid} -> null"
        ));
        std::ptr::null_mut()
    }
}

impl IUnitHandlerTrait for ComponentHandler {
    unsafe fn notifyUnitSelection(&self, unit_id: UnitID) -> tresult {
        crate::trace::emit(&format!("UnitHandler::notifyUnitSelection unit={unit_id}"));
        self.push_notification(HostNotification::UnitSelection(unit_id));
        kResultOk
    }

    unsafe fn notifyProgramListChange(
        &self,
        list_id: ProgramListID,
        program_index: int32,
    ) -> tresult {
        crate::trace::emit(&format!(
            "UnitHandler::notifyProgramListChange list={list_id} idx={program_index}"
        ));
        self.push_notification(HostNotification::ProgramListChange {
            list_id,
            program_index,
        });
        kResultOk
    }
}

impl IUnitHandler2Trait for ComponentHandler {
    unsafe fn notifyUnitByBusChange(&self) -> tresult {
        crate::trace::emit("UnitHandler2::notifyUnitByBusChange");
        self.push_notification(HostNotification::UnitByBusChange);
        kResultOk
    }
}

/// Create a new IComponentHandler COM wrapper paired with its drain queue,
/// restart-flags accumulator, and side-band notification queue (the latter
/// surfaces IComponentHandler2 / IUnitHandler / IUnitHandler2 callbacks).
pub(crate) fn create_component_handler_with_notifications(
) -> (
    ComWrapper<ComponentHandler>,
    ParamQueue,
    RestartFlags,
    NotificationQueue,
) {
    let queue = Arc::new(Mutex::new(Vec::new()));
    let restart_flags = Arc::new(AtomicI32::new(0));
    let notifications = Arc::new(Mutex::new(Vec::new()));
    let handler = ComponentHandler {
        queue: Arc::clone(&queue),
        restart_flags: Arc::clone(&restart_flags),
        notifications: Arc::clone(&notifications),
    };
    (ComWrapper::new(handler), queue, restart_flags, notifications)
}

/// Drain all host notifications, returning them in arrival order.
pub(crate) fn drain_notifications(q: &NotificationQueue) -> Vec<HostNotification> {
    q.lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Drain all pending parameter changes, returning them in arrival order.
pub(crate) fn drain(queue: &ParamQueue) -> Vec<PendingParam> {
    queue
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Atomically read and clear the accumulated restart flags. Returns 0
/// when no restart was requested since the last drain.
pub(crate) fn drain_restart_flags(flags: &RestartFlags) -> i32 {
    flags.swap(0, Ordering::AcqRel)
}

#[cfg(test)]
mod handler_extensions_tests {
    use super::*;
    use vst3::Steinberg::Vst::{
        IComponentHandler2, IComponentHandler2Trait, IComponentHandler3, IComponentHandler3Trait,
        IUnitHandler, IUnitHandler2, IUnitHandler2Trait, IUnitHandlerTrait,
    };

    #[test]
    fn handler_exposes_component_handler_3() {
        let (wrapper, _q, _r, _n) = create_component_handler_with_notifications();
        let ptr = wrapper.to_com_ptr::<IComponentHandler3>();
        assert!(
            ptr.is_some(),
            "ComponentHandler must expose IComponentHandler3 — plugins QI for createContextMenu"
        );
    }

    #[test]
    fn component_handler_3_create_context_menu_returns_null() {
        let (wrapper, _q, _r, _n) = create_component_handler_with_notifications();
        let h3 = wrapper.to_com_ptr::<IComponentHandler3>().unwrap();
        let param: ParamID = 7;
        unsafe {
            let menu = h3.createContextMenu(std::ptr::null_mut(), &param as *const _);
            // Headless host doesn't supply menu items; null is correct.
            assert!(
                menu.is_null(),
                "headless host should return null context menu"
            );
        }
    }

    #[test]
    fn handler_exposes_component_handler_2() {
        let (wrapper, _q, _r, _n) = create_component_handler_with_notifications();
        let ptr = wrapper.to_com_ptr::<IComponentHandler2>();
        assert!(
            ptr.is_some(),
            "ComponentHandler must expose IComponentHandler2 — plugins QI for this to enable setDirty, requestOpenEditor, group-edit semantics"
        );
    }

    #[test]
    fn handler_exposes_unit_handler() {
        let (wrapper, _q, _r, _n) = create_component_handler_with_notifications();
        let ptr = wrapper.to_com_ptr::<IUnitHandler>();
        assert!(
            ptr.is_some(),
            "ComponentHandler must expose IUnitHandler — plugins use this to notify host of unit/program list changes (multi-timbral samplers rely on it)"
        );
    }

    #[test]
    fn handler_exposes_unit_handler_2() {
        let (wrapper, _q, _r, _n) = create_component_handler_with_notifications();
        let ptr = wrapper.to_com_ptr::<IUnitHandler2>();
        assert!(
            ptr.is_some(),
            "ComponentHandler must expose IUnitHandler2 — plugins use notifyUnitByBusChange for dynamic multi-bus topology changes"
        );
    }

    #[test]
    fn component_handler_2_set_dirty_drains_to_notifications() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let h2 = wrapper.to_com_ptr::<IComponentHandler2>().unwrap();
        unsafe {
            let _ = h2.setDirty(1);
        }
        let drained = drain_notifications(&notifications);
        assert!(
            drained.iter().any(|n| matches!(n, HostNotification::SetDirty(true))),
            "setDirty(true) must surface as HostNotification::SetDirty(true), got {drained:?}"
        );
    }

    #[test]
    fn component_handler_2_request_open_editor_drains() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let h2 = wrapper.to_com_ptr::<IComponentHandler2>().unwrap();
        let name = b"editor\0";
        unsafe {
            let _ = h2.requestOpenEditor(name.as_ptr() as *const i8);
        }
        let drained = drain_notifications(&notifications);
        assert!(
            drained.iter().any(|n| matches!(n, HostNotification::RequestOpenEditor(s) if s == "editor")),
            "requestOpenEditor must surface as HostNotification::RequestOpenEditor(name), got {drained:?}"
        );
    }

    #[test]
    fn component_handler_2_group_edit_drains() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let h2 = wrapper.to_com_ptr::<IComponentHandler2>().unwrap();
        unsafe {
            let _ = h2.startGroupEdit();
            let _ = h2.finishGroupEdit();
        }
        let drained = drain_notifications(&notifications);
        let starts = drained
            .iter()
            .filter(|n| matches!(n, HostNotification::StartGroupEdit))
            .count();
        let finishes = drained
            .iter()
            .filter(|n| matches!(n, HostNotification::FinishGroupEdit))
            .count();
        assert_eq!(starts, 1, "expected exactly one StartGroupEdit, drained={drained:?}");
        assert_eq!(finishes, 1, "expected exactly one FinishGroupEdit, drained={drained:?}");
    }

    #[test]
    fn unit_handler_notify_unit_selection_drains() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let uh = wrapper.to_com_ptr::<IUnitHandler>().unwrap();
        unsafe {
            let _ = uh.notifyUnitSelection(42);
        }
        let drained = drain_notifications(&notifications);
        assert!(
            drained.iter().any(|n| matches!(n, HostNotification::UnitSelection(42))),
            "notifyUnitSelection(42) must surface as UnitSelection(42), got {drained:?}"
        );
    }

    #[test]
    fn unit_handler_program_list_change_drains() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let uh = wrapper.to_com_ptr::<IUnitHandler>().unwrap();
        unsafe {
            let _ = uh.notifyProgramListChange(7, 13);
        }
        let drained = drain_notifications(&notifications);
        assert!(
            drained.iter().any(|n| matches!(n, HostNotification::ProgramListChange { list_id: 7, program_index: 13 })),
            "notifyProgramListChange(7,13) must surface, got {drained:?}"
        );
    }

    #[test]
    fn unit_handler_2_notify_unit_by_bus_change_drains() {
        let (wrapper, _q, _r, notifications) = create_component_handler_with_notifications();
        let uh2 = wrapper.to_com_ptr::<IUnitHandler2>().unwrap();
        unsafe {
            let _ = uh2.notifyUnitByBusChange();
        }
        let drained = drain_notifications(&notifications);
        assert!(
            drained.iter().any(|n| matches!(n, HostNotification::UnitByBusChange)),
            "notifyUnitByBusChange must surface, got {drained:?}"
        );
    }
}
