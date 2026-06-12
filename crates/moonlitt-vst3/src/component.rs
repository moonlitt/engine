//! IComponent lifecycle management
//!
//! Handles plugin creation, initialization, activation, and teardown.
//! Follows the VST3 hosting sequence:
//! `GetPluginFactory → enumerate classes → createInstance<IComponent>
//! → initialize → QI<IAudioProcessor> → QI/create IEditController
//! → setupProcessing → activateBuses → setActive → setProcessing`

use std::ffi::c_void;
use std::mem::MaybeUninit;

use vst3::Steinberg::Vst::{
    BusDirections_::*, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentTrait,
    IEditController, IMidiMapping, MediaTypes_::*, ProcessModes_::kRealtime, ProcessSetup,
    SymbolicSampleSizes_::kSample32,
};
use vst3::Steinberg::{
    kNotImplemented, kResultOk, FUnknown, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait,
    PClassInfo,
};
use vst3::{ComPtr, Interface};

use crate::component_handler::{
    create_component_handler_with_notifications, ComponentHandler, NotificationQueue, ParamQueue,
    RestartFlags,
};
use crate::connection_bridge::BridgePair;
use crate::host::HostApp;
use crate::module::{GetFactoryFn, Module};
use crate::{Error, Result};

/// Information about a class discovered in a plugin factory.
#[derive(Debug, Clone)]
pub(crate) struct ClassInfo {
    pub name: String,
    pub category: String,
    pub cid: [u8; 16],
    /// Populated when the factory implements IPluginFactory2 (PClassInfo2).
    pub subcategories: Option<String>,
    pub vendor: Option<String>,
    pub version: Option<String>,
}

/// Public-facing topology of one audio bus (input or output). Built once
/// at load time so that consumers can ask the plugin "what do you
/// expose?" without going through trace logs or raw COM calls.
#[derive(Debug, Clone)]
pub(crate) struct AudioBusTopology {
    pub name: String,
    pub channel_count: u32,
    pub is_main: bool,
    pub default_active: bool,
}

/// Read all audio buses on the given direction into a Vec.
pub(crate) fn probe_audio_buses(
    component: &ComPtr<IComponent>,
    direction: i32,
) -> Vec<AudioBusTopology> {
    use vst3::Steinberg::Vst::{BusInfo, BusInfo_::BusFlags_};

    let count = unsafe { component.getBusCount(kAudio as i32, direction) };
    let mut buses = Vec::with_capacity(count.max(0) as usize);

    for i in 0..count {
        let mut info = MaybeUninit::<BusInfo>::uninit();
        if unsafe { component.getBusInfo(kAudio as i32, direction, i, info.as_mut_ptr()) }
            != kResultOk
        {
            continue;
        }
        let info = unsafe { info.assume_init() };
        buses.push(AudioBusTopology {
            name: bus_name_to_string(&info.name),
            channel_count: info.channelCount as u32,
            is_main: info.busType == 0,
            default_active: info.flags & BusFlags_::kDefaultActive != 0,
        });
    }

    buses
}

fn bus_name_to_string(s: &[u16; 128]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(128);
    String::from_utf16_lossy(&s[..end])
}

/// A fully loaded and activated VST3 plugin.
pub(crate) struct LoadedPlugin {
    pub component: ComPtr<IComponent>,
    pub processor: ComPtr<IAudioProcessor>,
    pub controller: Option<ComPtr<IEditController>>,
    pub class_info: ClassInfo,
    /// Pending controller→processor parameter changes. Drained on each
    /// render call and injected into ProcessData::inputParameterChanges.
    /// `None` when the plugin has no separate controller.
    pub param_queue: Option<ParamQueue>,
    /// OR-accumulator for restartComponent flags requested by the
    /// controller. Read-and-cleared at the start of each render.
    pub restart_flags: Option<RestartFlags>,
    /// Side-band host notifications (setDirty, requestOpenEditor, unit
    /// selection, program-list changes) the plug-in pushes through
    /// IComponentHandler2 / IUnitHandler / IUnitHandler2.
    pub notifications: Option<NotificationQueue>,
    /// Keeps the IComponentHandler COM wrapper alive for the plugin's
    /// lifetime — the controller stores a raw pointer to it via
    /// setComponentHandler. `None` mirrors `param_queue`.
    pub _component_handler: Option<vst3::ComWrapper<ComponentHandler>>,
    /// Keeps the IConnectionPoint trace bridge alive when tracing is on.
    /// `None` for direct (non-traced) connections.
    pub _connection_bridge: Option<BridgePair>,
    /// Plug-in's IMidiMapping if it implements one. Used to translate
    /// incoming MIDI controller events into ParamID-keyed parameter
    /// changes, per VST3 spec.
    pub midi_mapping: Option<ComPtr<IMidiMapping>>,
    /// Keeps the shared library loaded for the plugin's lifetime.
    pub _module: Module,
}

/// Load a VST3 plugin from a loaded module.
///
/// Performs the full lifecycle:
///   factory → enumerate → createInstance → initialize → QI
///   → setupProcessing → activateBuses → setActive → setProcessing
pub(crate) fn load_plugin(
    module: Module,
    class_id: &[u8; 16],
    host: &vst3::ComWrapper<HostApp>,
    sample_rate: f64,
    buffer_size: usize,
) -> Result<LoadedPlugin> {
    // 1. Call factory_fn() to get IPluginFactory
    let factory = get_factory(module.factory_fn)?;

    // 2. Find the class info for validation
    let class_info = find_class(&factory, class_id)?;
    crate::trace::emit(&format!(
        "load_plugin: class=\"{}\" category=\"{}\"",
        class_info.name, class_info.category
    ));

    // 3. createInstance with class_id for IComponent
    let component = create_component(&factory, class_id)?;
    crate::trace::emit("load_plugin: createInstance(IComponent) ok");

    // 4. initialize(host as FUnknown)
    initialize_component(&component, host)?;
    crate::trace::emit("load_plugin: component.initialize ok");

    // 5. QueryInterface for IAudioProcessor
    let processor = query_audio_processor(&component)?;
    crate::trace::emit("load_plugin: QI<IAudioProcessor> ok");

    // 6. Try to get IEditController
    let controller = get_edit_controller(&component, &factory, host);
    crate::trace::emit(&format!(
        "load_plugin: controller={}",
        if controller.is_some() { "ok" } else { "none" }
    ));

    // 6a. Connect component ↔ controller via IConnectionPoint. Plugins with
    // separate component/controller objects (Spectrasonics, NI, etc.) use
    // IMessage to exchange patch paths, license info, sample library refs.
    // Without this, internal messaging is silently dropped and patches fail
    // to load. When tracing is enabled, a relay sits in between to log
    // every message that crosses the wire.
    let connection_bridge = connect_component_controller(&component, controller.as_ref());

    // 6b. Sync component state → controller. Without this, controllers of
    // sampler-style plugins (Keyscape, Kontakt, sfizz) start with a stale
    // baseline and parameter writes go to the wrong slot.
    sync_component_state(&component, controller.as_ref());

    // 6c. Install our IComponentHandler so the controller can notify us about
    // parameter edits. Without this, performEdit calls go nowhere — the
    // processor never receives controller-side parameter changes. The
    // handler also exposes IComponentHandler2, IUnitHandler, IUnitHandler2
    // — plug-ins QI for these to surface setDirty / requestOpenEditor and
    // unit/program list changes.
    let (component_handler, param_queue, restart_flags, notifications) =
        match controller.as_ref() {
            Some(ctrl) => install_component_handler(ctrl),
            None => (None, None, None, None),
        };

    // 6d. QI the controller for IMidiMapping so we can translate incoming
    // MIDI controller events into parameter changes per the VST3 spec.
    // Plug-ins that don't expose IMidiMapping continue to receive raw
    // CC events through the event stream.
    let midi_mapping = controller.as_ref().and_then(|c| c.cast::<IMidiMapping>());
    crate::trace::emit(&format!(
        "load_plugin: IMidiMapping={}",
        if midi_mapping.is_some() { "available" } else { "absent" }
    ));

    // 7. setupProcessing
    setup_processing(&processor, sample_rate, buffer_size)?;
    crate::trace::emit(&format!(
        "load_plugin: setupProcessing(sr={sample_rate} block={buffer_size}) ok"
    ));

    // 7a. Negotiate bus arrangements. Host proposes stereo on every audio
    // bus; plugins that need other layouts (mono synth, multi-out sampler)
    // may renegotiate at this point. Failures are non-fatal — plugin keeps
    // its default layout.
    negotiate_bus_arrangements(&processor, &component);

    // 8. Activate buses
    activate_buses(&component)?;
    if crate::trace::enabled() {
        let n_a_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };
        let n_a_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };
        let n_e_in = unsafe { component.getBusCount(kEvent as i32, kInput as i32) };
        let n_e_out = unsafe { component.getBusCount(kEvent as i32, kOutput as i32) };
        crate::trace::emit(&format!(
            "load_plugin: buses audio_in={n_a_in} audio_out={n_a_out} event_in={n_e_in} event_out={n_e_out}"
        ));
        log_bus_details(&component, kAudio as i32, kOutput as i32, n_a_out, "audio_out");
        log_bus_details(&component, kAudio as i32, kInput as i32, n_a_in, "audio_in");
        log_bus_details(&component, kEvent as i32, kInput as i32, n_e_in, "event_in");
    }

    // 9. setActive(true)
    let result = unsafe { component.setActive(1) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }
    crate::trace::emit("load_plugin: setActive(1) ok");

    // 10. setProcessing(true)
    let result = unsafe { processor.setProcessing(1) };
    // Some plugins return kNotImplemented for setProcessing, which is OK
    if result != kResultOk && result != kNotImplemented {
        return Err(Error::PluginError(result));
    }
    crate::trace::emit(&format!(
        "load_plugin: setProcessing(1) -> {}",
        if result == kResultOk { "ok" } else { "kNotImplemented" }
    ));

    Ok(LoadedPlugin {
        component,
        processor,
        controller,
        class_info,
        param_queue,
        restart_flags,
        notifications,
        _component_handler: component_handler,
        _connection_bridge: connection_bridge,
        midi_mapping,
        _module: module,
    })
}

/// Hand the controller our IComponentHandler implementation. Returns the COM
/// wrapper (caller must keep it alive while the plugin is loaded), the
/// shared queue used to read back pending parameter changes, and the
/// restart-flags accumulator the controller writes when it asks for a
/// component reload / latency change / param refresh.
fn install_component_handler(
    controller: &ComPtr<IEditController>,
) -> (
    Option<vst3::ComWrapper<ComponentHandler>>,
    Option<ParamQueue>,
    Option<RestartFlags>,
    Option<NotificationQueue>,
) {
    use vst3::Steinberg::Vst::{IComponentHandler, IEditControllerTrait};

    let (wrapper, queue, restart_flags, notifications) =
        create_component_handler_with_notifications();
    let Some(handler_ptr) = wrapper.to_com_ptr::<IComponentHandler>() else {
        return (None, None, None, None);
    };

    let result = unsafe { controller.setComponentHandler(handler_ptr.as_ptr()) };
    if result != kResultOk {
        // Plugin refused our handler — extremely rare. Drop all four so we
        // don't pretend to capture edits or restart requests we'll never see.
        return (None, None, None, None);
    }

    (Some(wrapper), Some(queue), Some(restart_flags), Some(notifications))
}

/// Get IPluginFactory from the factory function pointer.
fn get_factory(factory_fn: GetFactoryFn) -> Result<ComPtr<IPluginFactory>> {
    let raw = unsafe { factory_fn() };
    if raw.is_null() {
        return Err(Error::LoadFailed("GetPluginFactory returned null".into()));
    }
    unsafe { ComPtr::from_raw(raw as *mut IPluginFactory) }
        .ok_or_else(|| Error::LoadFailed("null factory pointer".into()))
}

/// Enumerate classes in the factory, looking for a specific class ID.
fn find_class(factory: &ComPtr<IPluginFactory>, class_id: &[u8; 16]) -> Result<ClassInfo> {
    let count = unsafe { factory.countClasses() };
    let factory2 = factory.cast::<vst3::Steinberg::IPluginFactory2>();

    for i in 0..count {
        let mut info = MaybeUninit::<PClassInfo>::uninit();
        if unsafe { factory.getClassInfo(i, info.as_mut_ptr()) } != kResultOk {
            continue;
        }
        let info = unsafe { info.assume_init() };

        let cid = cid_to_bytes(&info.cid);
        if cid == *class_id {
            let (sub, vendor, version) = read_pclassinfo2(factory2.as_ref(), i);
            return Ok(ClassInfo {
                name: cstr_from_fixed(&info.name),
                category: cstr_from_fixed_i8(&info.category),
                cid,
                subcategories: sub,
                vendor,
                version,
            });
        }
    }

    Err(Error::Other("class not found in factory".into()))
}

/// Enumerate all Audio Module Classes in a factory (used by scanner).
pub(crate) fn enumerate_audio_classes(module: &Module) -> Result<Vec<ClassInfo>> {
    let factory = get_factory(module.factory_fn)?;
    let factory2 = factory.cast::<vst3::Steinberg::IPluginFactory2>();
    let count = unsafe { factory.countClasses() };
    let mut classes = Vec::new();

    for i in 0..count {
        let mut info = MaybeUninit::<PClassInfo>::uninit();
        if unsafe { factory.getClassInfo(i, info.as_mut_ptr()) } != kResultOk {
            continue;
        }
        let info = unsafe { info.assume_init() };

        let category = cstr_from_fixed_i8(&info.category);
        if category.contains("Audio") {
            let (sub, vendor, version) = read_pclassinfo2(factory2.as_ref(), i);
            classes.push(ClassInfo {
                name: cstr_from_fixed(&info.name),
                category,
                cid: cid_to_bytes(&info.cid),
                subcategories: sub,
                vendor,
                version,
            });
        }
    }

    Ok(classes)
}

/// Read PClassInfo2 fields for class `index` if the factory implements
/// IPluginFactory2. Returns (None, None, None) on legacy factories.
fn read_pclassinfo2(
    factory2: Option<&ComPtr<vst3::Steinberg::IPluginFactory2>>,
    index: i32,
) -> (Option<String>, Option<String>, Option<String>) {
    use vst3::Steinberg::{IPluginFactory2Trait, PClassInfo2};

    let Some(f2) = factory2 else {
        return (None, None, None);
    };
    let mut info2 = MaybeUninit::<PClassInfo2>::uninit();
    if unsafe { f2.getClassInfo2(index, info2.as_mut_ptr()) } != kResultOk {
        return (None, None, None);
    }
    let info2 = unsafe { info2.assume_init() };
    (
        Some(cstr_from_fixed_i8(&info2.subCategories)),
        Some(cstr_from_fixed_i8(&info2.vendor)),
        Some(cstr_from_fixed_i8(&info2.version)),
    )
}

/// Create an IComponent instance from the factory.
fn create_component(
    factory: &ComPtr<IPluginFactory>,
    class_id: &[u8; 16],
) -> Result<ComPtr<IComponent>> {
    let mut obj: *mut c_void = std::ptr::null_mut();
    let result = unsafe {
        factory.createInstance(
            class_id.as_ptr() as *const _ as *const i8,
            IComponent::IID.as_ptr() as *const i8,
            &mut obj,
        )
    };

    if result != kResultOk || obj.is_null() {
        return Err(Error::PluginError(result));
    }

    unsafe { ComPtr::from_raw(obj as *mut IComponent) }
        .ok_or(Error::InterfaceNotFound("IComponent"))
}

/// Initialize the component with the host context.
fn initialize_component(
    component: &ComPtr<IComponent>,
    host: &vst3::ComWrapper<HostApp>,
) -> Result<()> {
    // Get the IHostApplication ComPtr, then get a raw FUnknown pointer from it.
    use vst3::Steinberg::Vst::IHostApplication;
    let host_ptr = host
        .to_com_ptr::<IHostApplication>()
        .ok_or(Error::Other("failed to get host IHostApplication".into()))?;

    // IHostApplication inherits from FUnknown, so as_ptr gives us the interface pointer.
    // We pass it as *mut FUnknown to initialize().
    let result =
        unsafe { component.initialize(host_ptr.as_ptr() as *mut FUnknown) };

    if result != kResultOk {
        return Err(Error::PluginError(result));
    }

    Ok(())
}

/// QueryInterface for IAudioProcessor from the component.
fn query_audio_processor(
    component: &ComPtr<IComponent>,
) -> Result<ComPtr<IAudioProcessor>> {
    component
        .cast::<IAudioProcessor>()
        .ok_or(Error::InterfaceNotFound("IAudioProcessor"))
}

/// Try to get IEditController, either as same object (QI) or separate class.
fn get_edit_controller(
    component: &ComPtr<IComponent>,
    factory: &ComPtr<IPluginFactory>,
    host: &vst3::ComWrapper<HostApp>,
) -> Option<ComPtr<IEditController>> {
    // First try: QI component for IEditController (same object)
    if let Some(ctrl) = component.cast::<IEditController>() {
        return Some(ctrl);
    }

    // Second try: get separate controller class ID
    let mut controller_cid = [0i8; 16];
    let result = unsafe { component.getControllerClassId(&mut controller_cid) };
    if result != kResultOk {
        return None;
    }

    // Create the controller
    let cid_bytes: [u8; 16] = controller_cid.map(|b| b as u8);
    let mut obj: *mut c_void = std::ptr::null_mut();
    let result = unsafe {
        factory.createInstance(
            cid_bytes.as_ptr() as *const i8,
            IEditController::IID.as_ptr() as *const i8,
            &mut obj,
        )
    };

    if result != kResultOk || obj.is_null() {
        return None;
    }

    let ctrl = unsafe { ComPtr::from_raw(obj as *mut IEditController) }?;

    // Initialize the separate controller
    use vst3::Steinberg::Vst::IHostApplication;
    let host_ptr = host.to_com_ptr::<IHostApplication>()?;

    let result =
        unsafe { ctrl.initialize(host_ptr.as_ptr() as *mut FUnknown) };

    if result == kResultOk {
        Some(ctrl)
    } else {
        None
    }
}

/// Connect component and controller via their IConnectionPoint interfaces, so
/// they can exchange IMessage notifications. Single-component plugins (where
/// component and controller are the same object) skip silently — connect()
/// would still succeed but is meaningless.
///
/// Returns Some(BridgePair) when tracing is enabled (caller must keep it
/// alive — both sides hold raw pointers into the bridges).
fn connect_component_controller(
    component: &ComPtr<IComponent>,
    controller: Option<&ComPtr<IEditController>>,
) -> Option<BridgePair> {
    use vst3::Steinberg::Vst::{IConnectionPoint, IConnectionPointTrait};

    let ctrl = controller?;

    let cp_comp = component.cast::<IConnectionPoint>()?;
    let cp_ctrl = ctrl.cast::<IConnectionPoint>()?;

    // If both casts produced the same underlying object, this is a
    // single-component plugin — connection is unnecessary and could form a
    // self-loop on aggressive impls.
    if std::ptr::eq(cp_comp.as_ptr(), cp_ctrl.as_ptr()) {
        return None;
    }

    if crate::trace::enabled() {
        crate::connection_bridge::install(&cp_comp, &cp_ctrl)
    } else {
        unsafe {
            let _ = cp_comp.connect(cp_ctrl.as_ptr());
            let _ = cp_ctrl.connect(cp_comp.as_ptr());
        }
        None
    }
}

/// Sync the component's current state into the controller so the latter knows
/// the audio engine's baseline. Standard VST3 host flow per the SDK examples.
/// Non-fatal: many plugins return empty state or kNotImplemented.
fn sync_component_state(
    component: &ComPtr<IComponent>,
    controller: Option<&ComPtr<IEditController>>,
) {
    use vst3::Steinberg::Vst::IEditControllerTrait;

    let Some(ctrl) = controller else { return };

    let mut write_stream = crate::stream::MemoryStream::new_writable();
    let result = unsafe { component.getState(write_stream.as_ibstream_ptr()) };
    if result != kResultOk {
        return;
    }

    let data = write_stream.data().to_vec();
    if data.is_empty() {
        return;
    }

    let mut read_stream = crate::stream::MemoryStream::from_data(data);
    let _ = unsafe { ctrl.setComponentState(read_stream.as_ibstream_ptr()) };
}

/// Propose a stereo SpeakerArrangement for every audio bus the plugin
/// exposes. Plugin can renegotiate or accept silently. Result code is
/// ignored — plugin's own default layout is the fallback.
fn negotiate_bus_arrangements(
    processor: &ComPtr<IAudioProcessor>,
    component: &ComPtr<IComponent>,
) {
    use vst3::Steinberg::Vst::{SpeakerArr, SpeakerArrangement};

    let num_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };
    let num_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };

    let mut inputs: Vec<SpeakerArrangement> = vec![SpeakerArr::kStereo; num_in as usize];
    let mut outputs: Vec<SpeakerArrangement> = vec![SpeakerArr::kStereo; num_out as usize];

    let in_ptr = if inputs.is_empty() {
        std::ptr::null_mut()
    } else {
        inputs.as_mut_ptr()
    };
    let out_ptr = if outputs.is_empty() {
        std::ptr::null_mut()
    } else {
        outputs.as_mut_ptr()
    };

    let r = unsafe { processor.setBusArrangements(in_ptr, num_in, out_ptr, num_out) };
    crate::trace::emit(&format!(
        "load_plugin: setBusArrangements(in={num_in}, out={num_out}, layout=stereo) -> 0x{:08X}",
        r as u32
    ));
}

/// Log details (name, type, channels, default-active flag) for each bus in
/// a given direction. Used under MOONLITT_VST3_TRACE to understand plugin
/// I/O topology — multi-out samplers expose many buses and the host needs
/// to know which is "Main" vs "Aux".
fn log_bus_details(
    component: &ComPtr<IComponent>,
    media_type: i32,
    direction: i32,
    count: i32,
    label: &str,
) {
    use vst3::Steinberg::Vst::{BusInfo, BusInfo_::BusFlags_};

    for i in 0..count {
        let mut info = MaybeUninit::<BusInfo>::uninit();
        let r = unsafe { component.getBusInfo(media_type, direction, i, info.as_mut_ptr()) };
        if r != kResultOk {
            crate::trace::emit(&format!(
                "{label}[{i}] getBusInfo -> 0x{:08X}",
                r as u32
            ));
            continue;
        }
        let info = unsafe { info.assume_init() };
        let name = bus_name_to_string(&info.name);
        let bus_type = if info.busType == 0 { "Main" } else { "Aux" };
        let default_active = info.flags & BusFlags_::kDefaultActive != 0;
        crate::trace::emit(&format!(
            "{label}[{i}] name=\"{name}\" type={bus_type} channels={} default_active={default_active}",
            info.channelCount
        ));
    }
}

/// Setup processing parameters on the audio processor.
fn setup_processing(
    processor: &ComPtr<IAudioProcessor>,
    sample_rate: f64,
    buffer_size: usize,
) -> Result<()> {
    let mut setup = ProcessSetup {
        processMode: kRealtime as i32,
        symbolicSampleSize: kSample32 as i32,
        maxSamplesPerBlock: buffer_size as i32,
        sampleRate: sample_rate,
    };

    let result = unsafe { processor.setupProcessing(&mut setup) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }
    Ok(())
}

/// Activate all audio and event buses.
fn activate_buses(component: &ComPtr<IComponent>) -> Result<()> {
    let num_audio_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };
    let num_audio_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };
    let num_event_in = unsafe { component.getBusCount(kEvent as i32, kInput as i32) };

    for i in 0..num_audio_out {
        unsafe { component.activateBus(kAudio as i32, kOutput as i32, i, 1) };
    }
    for i in 0..num_audio_in {
        unsafe { component.activateBus(kAudio as i32, kInput as i32, i, 1) };
    }
    for i in 0..num_event_in {
        unsafe { component.activateBus(kEvent as i32, kInput as i32, i, 1) };
    }

    Ok(())
}

/// Deactivate and terminate a loaded plugin.
pub(crate) fn unload_plugin(plugin: &mut LoadedPlugin) {
    // setProcessing(false)
    unsafe { plugin.processor.setProcessing(0) };

    // setActive(false)
    unsafe { plugin.component.setActive(0) };

    // If controller is a separate object from component, terminate it
    if let Some(ref ctrl) = plugin.controller {
        // Check if controller is same object as component by comparing
        // raw interface pointers via QI for FUnknown
        let comp_ptr = plugin.component.cast::<FUnknown>().map(|p| p.as_ptr());
        let ctrl_ptr = ctrl.cast::<FUnknown>().map(|p| p.as_ptr());

        let same_object = match (comp_ptr, ctrl_ptr) {
            (Some(c), Some(e)) => std::ptr::eq(c, e),
            _ => false,
        };

        if !same_object {
            unsafe { ctrl.terminate() };
        }
    }

    // terminate component
    unsafe { plugin.component.terminate() };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert TUID (c_char array) to [u8; 16].
fn cid_to_bytes(cid: &[std::ffi::c_char; 16]) -> [u8; 16] {
    cid.map(|b| b as u8)
}

/// Read a null-terminated string from a fixed-size i8/char8 buffer.
fn cstr_from_fixed_i8(buf: &[std::ffi::c_char]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    buf[..end].iter().map(|&c| c as u8 as char).collect()
}

/// Read a null-terminated string from a fixed-size i8 buffer (same as above, for name field).
fn cstr_from_fixed(buf: &[std::ffi::c_char]) -> String {
    cstr_from_fixed_i8(buf)
}
