//! IConnectionPoint message bridge for tracing.
//!
//! Component and controller normally connect directly to each other and
//! exchange `IMessage` notifications point-to-point. To observe what
//! crosses that wire (which patches a sampler is asking for, what licensing
//! info is being negotiated, etc.) we put a tracing relay in between:
//!
//! ```text
//!   [Component] --connect--> [Bridge_C2E] --notify--> [Controller]
//!   [Controller] --connect--> [Bridge_E2C] --notify--> [Component]
//! ```
//!
//! Each bridge logs the message ID then forwards `notify()` to its target.
//! Connect/disconnect are handled directly without forwarding (the plugin
//! doesn't need to see our bridge connecting to itself).
//!
//! Only used when `MOONLITT_VST3_TRACE` is enabled. Without tracing, the
//! caller wires component ↔ controller directly to keep the hot path lean.

use std::sync::Mutex;

use vst3::Steinberg::Vst::{IConnectionPoint, IConnectionPointTrait, IMessage};
use vst3::Steinberg::{kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

/// One direction of the bridge: receives notify() calls and forwards them
/// to a stored target IConnectionPoint, logging the message ID along the
/// way. The label distinguishes the two directions in trace output.
pub(crate) struct ConnectionBridge {
    /// Where to forward `notify()` calls. Stored behind a mutex so we can
    /// install the target after both bridges are constructed (chicken/egg
    /// — each bridge's target is the OTHER bridge during setup).
    target: Mutex<Option<ComPtr<IConnectionPoint>>>,
    /// Trace label, e.g. "comp->ctrl" or "ctrl->comp".
    label: &'static str,
}

impl Class for ConnectionBridge {
    type Interfaces = (IConnectionPoint,);
}

impl IConnectionPointTrait for ConnectionBridge {
    unsafe fn connect(&self, _other: *mut IConnectionPoint) -> tresult {
        // The plugin calls connect() back on us when we connect to it. We
        // accept silently — our forwarding target is set externally.
        kResultOk
    }

    unsafe fn disconnect(&self, _other: *mut IConnectionPoint) -> tresult {
        if let Ok(mut t) = self.target.lock() {
            *t = None;
        }
        kResultOk
    }

    unsafe fn notify(&self, message: *mut IMessage) -> tresult {
        // Read message ID for trace.
        let msg_id = if message.is_null() {
            "<null>".to_string()
        } else {
            let id_ptr = ((*(*message).vtbl).getMessageID)(message);
            if id_ptr.is_null() {
                "<null-id>".to_string()
            } else {
                let cstr = std::ffi::CStr::from_ptr(id_ptr);
                cstr.to_string_lossy().into_owned()
            }
        };
        crate::trace::emit(&format!("CP[{}] notify msg=\"{}\"", self.label, msg_id));

        // Forward to real target.
        let target = match self.target.lock() {
            Ok(t) => t.clone(),
            Err(_) => None,
        };
        match target {
            Some(t) => t.notify(message),
            None => kResultOk,
        }
    }
}

pub(crate) struct BridgePair {
    /// Bridge component sees as its peer; logs and forwards to controller.
    /// Field is intentionally retained — the plugin holds a raw pointer to it.
    pub _bridge_for_comp: ComWrapper<ConnectionBridge>,
    /// Bridge controller sees as its peer; logs and forwards to component.
    pub _bridge_for_ctrl: ComWrapper<ConnectionBridge>,
}

/// Build a paired bridge that, once installed via `connect()` calls on the
/// component and controller, will log every message and forward it.
pub(crate) fn install(
    cp_comp: &ComPtr<IConnectionPoint>,
    cp_ctrl: &ComPtr<IConnectionPoint>,
) -> Option<BridgePair> {
    let bridge_for_comp = ComWrapper::new(ConnectionBridge {
        target: Mutex::new(None),
        label: "comp->ctrl",
    });
    let bridge_for_ctrl = ComWrapper::new(ConnectionBridge {
        target: Mutex::new(None),
        label: "ctrl->comp",
    });

    // Resolve the IConnectionPoint pointers.
    let bridge_for_comp_cp = bridge_for_comp.to_com_ptr::<IConnectionPoint>()?;
    let bridge_for_ctrl_cp = bridge_for_ctrl.to_com_ptr::<IConnectionPoint>()?;

    // Set forwarding targets: when component sends, we forward to controller
    // (and vice versa). ComWrapper derefs to &ConnectionBridge.
    if let Ok(mut t) = bridge_for_comp.target.lock() {
        *t = Some(cp_ctrl.clone());
    }
    if let Ok(mut t) = bridge_for_ctrl.target.lock() {
        *t = Some(cp_comp.clone());
    }

    // Connect: component's peer is bridge_for_comp (forwards to ctrl);
    // controller's peer is bridge_for_ctrl (forwards to comp).
    unsafe {
        let _ = cp_comp.connect(bridge_for_comp_cp.as_ptr());
        let _ = cp_ctrl.connect(bridge_for_ctrl_cp.as_ptr());
    }

    Some(BridgePair {
        _bridge_for_comp: bridge_for_comp,
        _bridge_for_ctrl: bridge_for_ctrl,
    })
}
