//! IComponent lifecycle management
//!
//! Handles plugin creation, initialization, activation, and teardown.
//! Follows the VST3 hosting sequence:
//!   GetPluginFactory → enumerate classes → createInstance<IComponent>
//!   → initialize → QI<IAudioProcessor> → QI/create IEditController
//!   → setupProcessing → activateBuses → setActive → setProcessing

use std::ffi::c_void;
use std::mem::MaybeUninit;

use vst3::Steinberg::Vst::{
    BusDirections_::*, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentTrait,
    IEditController, MediaTypes_::*, ProcessModes_::kRealtime, ProcessSetup,
    SymbolicSampleSizes_::kSample32,
};
use vst3::Steinberg::{
    kNotImplemented, kResultOk, FUnknown, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait,
    PClassInfo,
};
use vst3::{ComPtr, Interface};

use crate::host::HostApp;
use crate::module::GetFactoryFn;
use crate::{Error, Result};

/// Information about a class discovered in a plugin factory.
#[derive(Debug, Clone)]
pub(crate) struct ClassInfo {
    pub name: String,
    pub category: String,
    pub cid: [u8; 16],
}

/// A fully loaded and activated VST3 plugin.
pub(crate) struct LoadedPlugin {
    pub component: ComPtr<IComponent>,
    pub processor: ComPtr<IAudioProcessor>,
    pub controller: Option<ComPtr<IEditController>>,
    pub class_info: ClassInfo,
}

/// Load a VST3 plugin from a factory function.
///
/// Performs the full lifecycle:
///   factory → enumerate → createInstance → initialize → QI
///   → setupProcessing → activateBuses → setActive → setProcessing
pub(crate) fn load_plugin(
    factory_fn: GetFactoryFn,
    class_id: &[u8; 16],
    host: &vst3::ComWrapper<HostApp>,
    sample_rate: f64,
    buffer_size: usize,
) -> Result<LoadedPlugin> {
    // 1. Call factory_fn() to get IPluginFactory
    let factory = get_factory(factory_fn)?;

    // 2. Find the class info for validation
    let class_info = find_class(&factory, class_id)?;

    // 3. createInstance with class_id for IComponent
    let component = create_component(&factory, class_id)?;

    // 4. initialize(host as FUnknown)
    initialize_component(&component, host)?;

    // 5. QueryInterface for IAudioProcessor
    let processor = query_audio_processor(&component)?;

    // 6. Try to get IEditController
    let controller = get_edit_controller(&component, &factory, host);

    // 7. setupProcessing
    setup_processing(&processor, sample_rate, buffer_size)?;

    // 8. Activate buses
    activate_buses(&component)?;

    // 9. setActive(true)
    let result = unsafe { component.setActive(1) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }

    // 10. setProcessing(true)
    let result = unsafe { processor.setProcessing(1) };
    // Some plugins return kNotImplemented for setProcessing, which is OK
    if result != kResultOk && result != kNotImplemented {
        return Err(Error::PluginError(result));
    }

    Ok(LoadedPlugin {
        component,
        processor,
        controller,
        class_info,
    })
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

    for i in 0..count {
        let mut info = MaybeUninit::<PClassInfo>::uninit();
        if unsafe { factory.getClassInfo(i, info.as_mut_ptr()) } != kResultOk {
            continue;
        }
        let info = unsafe { info.assume_init() };

        let cid = cid_to_bytes(&info.cid);
        if cid == *class_id {
            return Ok(ClassInfo {
                name: cstr_from_fixed(&info.name),
                category: cstr_from_fixed_i8(&info.category),
                cid,
            });
        }
    }

    Err(Error::Other("class not found in factory".into()))
}

/// Enumerate all Audio Module Classes in a factory (used by scanner).
pub(crate) fn enumerate_audio_classes(factory_fn: GetFactoryFn) -> Result<Vec<ClassInfo>> {
    let factory = get_factory(factory_fn)?;
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
            classes.push(ClassInfo {
                name: cstr_from_fixed(&info.name),
                category,
                cid: cid_to_bytes(&info.cid),
            });
        }
    }

    Ok(classes)
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
