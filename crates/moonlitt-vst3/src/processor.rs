//! IAudioProcessor wrapper
//!
//! Handles audio processing: builds ProcessData with AudioBusBuffers and
//! IEventList, then calls IAudioProcessor::process().

use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirections_::*, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentTrait, IEventList, MediaTypes_::kAudio,
    ProcessData, ProcessModes_::kRealtime, SymbolicSampleSizes_::kSample32,
};
use vst3::Steinberg::kResultOk;
use vst3::ComPtr;

use crate::events::{create_event_list, MidiEvent};
use crate::{Error, Result};

/// Process one block of audio through the plugin.
///
/// Builds ProcessData with:
/// - Silent input bus(es) (for instruments that expect audio input)
/// - Output buses (first bus maps to the caller's left/right buffers)
/// - Input events from the provided MIDI events
///
/// This follows the same pattern as the C++ vst3_engine.cpp render().
pub(crate) fn process_audio(
    processor: &ComPtr<IAudioProcessor>,
    component: &ComPtr<IComponent>,
    left: &mut [f32],
    right: &mut [f32],
    events: &[MidiEvent],
) -> Result<()> {
    let num_frames = left.len().min(right.len());
    if num_frames == 0 {
        return Ok(());
    }

    let num_audio_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };
    let num_audio_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };

    // --- Input bus (silent) ---
    // Some plugins (e.g. Pianoteq) expect at least 1 audio input bus.
    let mut silent_left = vec![0.0f32; num_frames];
    let mut silent_right = vec![0.0f32; num_frames];
    let mut silent_ptrs: [*mut f32; 2] = [silent_left.as_mut_ptr(), silent_right.as_mut_ptr()];

    let mut input_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: u64::MAX, // all silent
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: silent_ptrs.as_mut_ptr(),
        },
    };

    // --- Output buses ---
    // First output bus writes to caller's L/R. Remaining buses get scratch buffers.
    const MAX_OUT_BUSES: usize = 16;
    let actual_out = (num_audio_out as usize).min(MAX_OUT_BUSES);

    let mut out_ptrs: [*mut f32; 2] = [left.as_mut_ptr(), right.as_mut_ptr()];

    // Create independent scratch buffers for each extra output bus.
    // Each bus needs its own pair of L/R buffers to avoid aliasing.
    let mut extra_scratches: Vec<(Vec<f32>, Vec<f32>)> = (1..actual_out)
        .map(|_| (vec![0.0f32; num_frames], vec![0.0f32; num_frames]))
        .collect();
    // Build pointer arrays for each extra bus (must live alongside the Vecs)
    let mut extra_ptrs: Vec<[*mut f32; 2]> = extra_scratches
        .iter_mut()
        .map(|(l, r)| [l.as_mut_ptr(), r.as_mut_ptr()])
        .collect();

    // Zero the output buffers
    left.fill(0.0);
    right.fill(0.0);

    // Build output bus array
    let mut output_buses: Vec<AudioBusBuffers> = Vec::with_capacity(actual_out);
    for i in 0..actual_out {
        let ptrs = if i == 0 {
            &mut out_ptrs as *mut [*mut f32; 2] as *mut *mut f32
        } else {
            &mut extra_ptrs[i - 1] as *mut [*mut f32; 2] as *mut *mut f32
        };
        output_buses.push(AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: ptrs,
            },
        });
    }

    // --- Event list ---
    let input_events = create_event_list(events);
    let input_events_ptr = input_events
        .to_com_ptr::<IEventList>()
        .ok_or(Error::Other("failed to create IEventList".into()))?;

    // --- Build ProcessData ---
    let mut data = ProcessData {
        processMode: kRealtime as i32,
        symbolicSampleSize: kSample32 as i32,
        numSamples: num_frames as i32,
        numInputs: if num_audio_in > 0 { 1 } else { 0 },
        numOutputs: actual_out as i32,
        inputs: if num_audio_in > 0 {
            &mut input_bus
        } else {
            std::ptr::null_mut()
        },
        outputs: if output_buses.is_empty() {
            std::ptr::null_mut()
        } else {
            output_buses.as_mut_ptr()
        },
        inputParameterChanges: std::ptr::null_mut(),
        outputParameterChanges: std::ptr::null_mut(),
        inputEvents: input_events_ptr.as_ptr(),
        outputEvents: std::ptr::null_mut(),
        processContext: std::ptr::null_mut(),
    };

    let result = unsafe { processor.process(&mut data) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }

    Ok(())
}
