//! IAudioProcessor wrapper
//!
//! Handles audio processing: builds ProcessData with AudioBusBuffers and
//! IEventList, then calls IAudioProcessor::process().

use vst3::ComPtr;
use vst3::Steinberg::kResultOk;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, BusDirections_::*, Chord, FrameRate, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentTrait, IEventList, IParameterChanges,
    MediaTypes_::kAudio, ProcessContext, ProcessContext_::StatesAndFlags_, ProcessData,
    ProcessModes_::kRealtime, SymbolicSampleSizes_::kSample32,
};

use crate::component_handler::PendingParam;
use crate::events::{create_event_list, drain_output_events, new_output_event_list, MidiEvent};
use crate::parameter_changes::{build_input_changes, drain_output, new_output_changes};
use crate::TransportContext;
use crate::{Error, Result};

/// One stereo output bus that the caller has provided buffers for. The
/// caller is responsible for buffer lifetime; the slices must live for
/// the duration of the `process_audio` call. Slices must be `num_frames`
/// long.
pub(crate) struct OutputBus<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

/// Process one block of audio through the plugin.
///
/// Builds ProcessData with:
/// - Silent input bus(es) (for instruments that expect audio input)
/// - One stereo output bus per entry in `outputs` — the caller chooses
///   what to back each bus with (e.g. their primary mix buffer for bus
///   0, pre-allocated scratch for the rest)
/// - Input events from the provided MIDI events
///
/// `outputs.len()` should match the plugin's audio-output bus count;
/// extra entries beyond what the plugin exposes are ignored, missing
/// entries are not surfaced (the plugin will write only as many buses
/// as we declare in `numOutputs`).
// The argument list mirrors the VST3 `ProcessData` layout one field at a
// time; collapsing it into a wrapper struct would just duplicate that
// FFI shape under a second name.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_audio(
    processor: &ComPtr<IAudioProcessor>,
    component: &ComPtr<IComponent>,
    outputs: &mut [OutputBus<'_>],
    events: &[MidiEvent],
    pending_params: &[PendingParam],
    output_params: &mut Vec<PendingParam>,
    output_events: &mut Vec<MidiEvent>,
    transport: &TransportContext,
    sample_rate: f64,
    silent_left: &mut [f32],
    silent_right: &mut [f32],
) -> Result<()> {
    let num_frames = outputs
        .iter()
        .map(|b| b.left.len().min(b.right.len()))
        .min()
        .unwrap_or(0);
    if num_frames == 0 || outputs.is_empty() {
        return Ok(());
    }

    let num_audio_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };
    let num_audio_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };

    // --- Input bus (silent) ---
    // Some plugins (e.g. Pianoteq) expect at least 1 audio input bus.
    // Silent buffers are pre-allocated by Vst3Plugin to avoid hot-path allocation.
    let mut silent_ptrs: [*mut f32; 2] = [silent_left.as_mut_ptr(), silent_right.as_mut_ptr()];

    let mut input_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: u64::MAX, // all silent
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: silent_ptrs.as_mut_ptr(),
        },
    };

    // --- Output buses ---
    // Cap at min(plugin-declared count, caller-provided count).
    let actual_out = (num_audio_out as usize).min(outputs.len());

    // Per-bus L/R pointer arrays that must live alongside `output_buses`
    // — each AudioBusBuffers stores a pointer into the corresponding
    // entry here. Pre-allocated to avoid resizing during fill.
    let mut bus_ptrs: Vec<[*mut f32; 2]> = Vec::with_capacity(actual_out);
    for bus in outputs.iter_mut().take(actual_out) {
        bus.left[..num_frames].fill(0.0);
        bus.right[..num_frames].fill(0.0);
        bus_ptrs.push([bus.left.as_mut_ptr(), bus.right.as_mut_ptr()]);
    }

    let mut output_buses: Vec<AudioBusBuffers> = Vec::with_capacity(actual_out);
    for bus_ptr in &mut bus_ptrs {
        let ptrs = bus_ptr as *mut [*mut f32; 2] as *mut *mut f32;
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

    // --- Parameter changes (controller→processor) ---
    // Wrappers must outlive the process() call; keep them bound here.
    let input_param_changes = build_input_changes(pending_params);
    let input_param_changes_ptr = input_param_changes
        .as_ref()
        .and_then(|w| w.as_com_ref::<IParameterChanges>())
        .map(|r| r.as_ptr())
        .unwrap_or(std::ptr::null_mut());

    // Output side: plugin writes parameter feedback (envelope follower,
    // LFO outputs, internal automation) here for the host to read.
    let output_param_changes = new_output_changes();
    let output_param_changes_ptr = output_param_changes
        .as_com_ref::<IParameterChanges>()
        .map(|r| r.as_ptr())
        .unwrap_or(std::ptr::null_mut());

    // --- Output event list (plugin writes, host drains) ---
    let output_event_list = new_output_event_list();
    let output_events_ptr = output_event_list
        .as_com_ref::<IEventList>()
        .map(|r| r.as_ptr())
        .unwrap_or(std::ptr::null_mut());

    // --- Process context (transport / playhead) ---
    let mut process_context = build_process_context(transport, sample_rate);

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
        inputParameterChanges: input_param_changes_ptr,
        outputParameterChanges: output_param_changes_ptr,
        inputEvents: input_events_ptr.as_ptr(),
        outputEvents: output_events_ptr,
        processContext: &mut process_context,
    };

    let result = unsafe { processor.process(&mut data) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }

    // Per-bus peak capture under trace. Useful for diagnosing multi-out
    // plugins that route their primary signal to a non-zero bus, where
    // the caller's L/R buffers stay silent even though the plugin is
    // producing audio.
    if crate::trace::enabled() {
        log_per_bus_peaks(&outputs[..actual_out], num_frames);
    }

    // Drain any feedback the plugin wrote into outputParameterChanges.
    *output_params = drain_output(&output_param_changes);
    // Drain any MIDI the plugin emitted (arpeggiator notes, etc.).
    *output_events = drain_output_events(&output_event_list);

    Ok(())
}

/// Aggregate peak per output bus and emit a single trace line per render
/// once the buffer carries non-trivial signal. We rate-limit by only
/// logging when the running max changes meaningfully — avoids flooding
/// the trace stream with zeros during silent regions.
fn log_per_bus_peaks(buses: &[OutputBus<'_>], num_frames: usize) {
    use std::sync::Mutex;
    static LAST_PEAKS: Mutex<Vec<f32>> = Mutex::new(Vec::new());

    let mut peaks = Vec::with_capacity(buses.len());
    for bus in buses {
        peaks.push(peak_pair(&bus.left[..num_frames], &bus.right[..num_frames]));
    }

    let bucket = |x: f32| -> u8 {
        if x < 1e-6 {
            0
        } else if x < 1e-3 {
            1
        } else if x < 0.1 {
            2
        } else if x < 1.0 {
            3
        } else {
            4
        }
    };

    let Ok(mut last) = LAST_PEAKS.lock() else {
        return;
    };
    let last_buckets: Vec<u8> = last.iter().map(|&p| bucket(p)).collect();
    let curr_buckets: Vec<u8> = peaks.iter().map(|&p| bucket(p)).collect();

    if last_buckets != curr_buckets {
        let parts: Vec<String> = peaks
            .iter()
            .enumerate()
            .map(|(i, p)| format!("bus{i}={:.4}", p))
            .collect();
        crate::trace::emit(&format!("process: peaks {}", parts.join(" ")));
        *last = peaks;
    }
}

fn peak_pair(l: &[f32], r: &[f32]) -> f32 {
    let lp = l.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
    let rp = r.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
    lp.max(rp)
}

/// Build a VST3 ProcessContext from our minimal TransportContext, with the
/// state flags advertising which fields the plugin can trust.
fn build_process_context(transport: &TransportContext, sample_rate: f64) -> ProcessContext {
    let mut state: u32 = StatesAndFlags_::kTempoValid
        | StatesAndFlags_::kTimeSigValid
        | StatesAndFlags_::kProjectTimeMusicValid;
    if transport.playing {
        state |= StatesAndFlags_::kPlaying;
    }

    // Convert sample-position → quarter-note position. 60s × bpm / 60 quarters
    // per minute means quarters = seconds × bpm / 60. Same algebra as the
    // VST3 SDK's HostContext example.
    let seconds = if sample_rate > 0.0 {
        transport.position_samples as f64 / sample_rate
    } else {
        0.0
    };
    let project_time_music = seconds * transport.tempo / 60.0;

    ProcessContext {
        state,
        sampleRate: sample_rate,
        projectTimeSamples: transport.position_samples,
        systemTime: 0,
        continousTimeSamples: transport.position_samples,
        projectTimeMusic: project_time_music,
        barPositionMusic: 0.0,
        cycleStartMusic: 0.0,
        cycleEndMusic: 0.0,
        tempo: transport.tempo,
        timeSigNumerator: transport.time_sig_num,
        timeSigDenominator: transport.time_sig_den,
        chord: Chord {
            keyNote: 0,
            rootNote: 0,
            chordMask: 0,
        },
        smpteOffsetSubframes: 0,
        frameRate: FrameRate {
            framesPerSecond: 0,
            flags: 0,
        },
        samplesToNextClock: 0,
    }
}

/// Process audio as an effect: feed real audio input, get processed output.
pub(crate) fn process_effect(
    processor: &ComPtr<IAudioProcessor>,
    component: &ComPtr<IComponent>,
    in_left: &[f32],
    in_right: &[f32],
    out_left: &mut [f32],
    out_right: &mut [f32],
) -> Result<()> {
    let num_frames = in_left
        .len()
        .min(in_right.len())
        .min(out_left.len())
        .min(out_right.len());
    if num_frames == 0 {
        return Ok(());
    }

    let num_audio_out = unsafe { component.getBusCount(kAudio as i32, kOutput as i32) };
    let num_audio_in = unsafe { component.getBusCount(kAudio as i32, kInput as i32) };

    // --- Input bus (real audio) ---
    // SAFETY: VST3 API declares channelBuffers32 as *mut but the spec guarantees
    // the plugin will not modify input buffers. We cast away const to satisfy
    // the API without a hot-path allocation.
    let in_left_ptr = in_left.as_ptr() as *mut f32;
    let in_right_ptr = in_right.as_ptr() as *mut f32;
    let mut input_ptrs: [*mut f32; 2] = [in_left_ptr, in_right_ptr];

    let mut input_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0, // NOT silent — real audio
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: input_ptrs.as_mut_ptr(),
        },
    };

    // --- Output bus ---
    out_left.fill(0.0);
    out_right.fill(0.0);
    let mut out_ptrs: [*mut f32; 2] = [out_left.as_mut_ptr(), out_right.as_mut_ptr()];

    let mut output_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: out_ptrs.as_mut_ptr(),
        },
    };

    let mut data = ProcessData {
        processMode: kRealtime as i32,
        symbolicSampleSize: kSample32 as i32,
        numSamples: num_frames as i32,
        numInputs: if num_audio_in > 0 { 1 } else { 0 },
        numOutputs: if num_audio_out > 0 { 1 } else { 0 },
        inputs: if num_audio_in > 0 {
            &mut input_bus
        } else {
            std::ptr::null_mut()
        },
        outputs: &mut output_bus,
        inputParameterChanges: std::ptr::null_mut(),
        outputParameterChanges: std::ptr::null_mut(),
        inputEvents: std::ptr::null_mut(), // no MIDI events for effects
        outputEvents: std::ptr::null_mut(),
        processContext: std::ptr::null_mut(),
    };

    let result = unsafe { processor.process(&mut data) };
    if result != kResultOk {
        return Err(Error::PluginError(result));
    }

    Ok(())
}
