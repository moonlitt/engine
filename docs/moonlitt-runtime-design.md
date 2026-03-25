# Moonlitt Runtime — 实时音频运行时设计

## 概述

`moonlitt-runtime` 是 moonlitt 生态的实时调度层，回答三个问题：
- **什么时候渲染？** — 音频硬件回调驱动
- **从哪来？** — MIDI 键盘、MIDI 文件、程序化 API
- **到哪去？** — 音箱/耳机（cpal）

与 `moonlitt-engine`（合成层，回答"渲染什么声音"）配合，构成完整的音频平台。

## 核心约束

- **音频线程永不阻塞** — 零 mutex、零内存分配
- **无锁通信** — 主线程与音频线程通过 SPSC 环形缓冲区通信
- **Engine 保持同步** — Runtime 拥有 Engine，在音频线程中调用 render()
- **所有输入源统一** — MIDI 键盘、序列器、用户 API 产生同一种事件类型

## 架构

```
主线程                              音频线程 (cpal 回调)
──────                              ─────────────────
                                     被音频硬件驱动
                                       │
rt.note_on(60) ─────►  ┌──────────┐   │
                        │  rtrb    │   │
MIDI 键盘 (midir) ───►  │  无锁    │ ──► drain events
                        │  事件    │   │   ↓
序列器 (midly) ──────►  │  队列    │   │  engine.note_on/off/cc...
                        └──────────┘   │   ↓
                                       │  engine.render(left, right)
                                       │   ↓
                                       │  cpal 音频输出 → 音箱
```

## 公开 API

```rust
use moonlitt_runtime::{Runtime, RuntimeConfig};
use moonlitt_engine::Engine;
use std::time::Duration;

// 创建引擎
let mut engine = Engine::new(44100, 256);
engine.load("Pianoteq 9.vst3")?;

// 创建运行时
let config = RuntimeConfig {
    buffer_size: 256,
    event_queue_capacity: 1024,
};
let mut rt = Runtime::new(engine, config)?;

// ═══════════════════════════════════════
// 音频输出
// ═══════════════════════════════════════

rt.start()?;                         // 启动音频输出
rt.stop()?;                          // 停止音频输出

// ═══════════════════════════════════════
// 方式 1: 程序化 MIDI（Piano Block 场景）
// ═══════════════════════════════════════

rt.note_on(0, 60, 100);             // 线程安全，无锁入队
rt.note_off(0, 60);
rt.cc(0, 64, 127);                  // sustain pedal
rt.pitch_bend(0, 4096);
rt.program_change(0, 5);
rt.all_notes_off();
rt.set_volume(0.8);

// ═══════════════════════════════════════
// 方式 2: MIDI 键盘（实时演奏）
// ═══════════════════════════════════════

let devices = rt.list_midi_inputs()?;
rt.connect_midi_input(&devices[0])?; // 键盘事件自动入队
rt.disconnect_midi_input()?;

// ═══════════════════════════════════════
// 方式 3: MIDI 文件回放（编曲播放）
// ═══════════════════════════════════════

rt.load_midi("song.mid")?;
rt.play();                           // 按时间线播放
rt.pause();
rt.seek(Duration::from_secs(30));
rt.set_tempo(120.0);                 // BPM
rt.set_loop(true);
rt.play();
rt.stop_playback();

// ═══════════════════════════════════════
// 查询
// ═══════════════════════════════════════

rt.is_playing() -> bool;
rt.position() -> Duration;
rt.tempo() -> f64;

// ═══════════════════════════════════════
// 生命周期
// ═══════════════════════════════════════

let engine = rt.shutdown();          // 归还 Engine 所有权
```

## 事件系统

```rust
/// 统一事件类型。所有输入源产生同一种事件。
#[derive(Debug, Clone, Copy)]
pub enum AudioEvent {
    // MIDI 消息
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
    AllNotesOff,

    // 引擎控制
    SetVolume(f32),

    // 传输控制（序列器 → 音频线程）
    Stop,
}
```

**大小：** AudioEvent 必须是 `Copy`，固定大小，适合无锁队列传输。

**队列：** `rtrb::RingBuffer<AudioEvent>` — SPSC（单生产者单消费者）。
多个输入源（用户 API、MIDI 键盘、序列器）都在主线程运行，共享 Producer 端。
音频线程持有 Consumer 端。

## 内部模块

### audio_output.rs — cpal 音频输出

```rust
pub(crate) struct AudioOutput {
    stream: cpal::Stream,
}

impl AudioOutput {
    /// 创建音频流。回调中：
    /// 1. 从 event_queue consumer 排空所有事件
    /// 2. 对每个事件调用 engine 的对应方法
    /// 3. 调用 engine.render() 填充输出 buffer
    pub fn new(
        engine: Arc<Mutex<Engine>>,  // 见线程安全说明
        consumer: rtrb::Consumer<AudioEvent>,
        buffer_size: usize,
    ) -> Result<Self>;

    pub fn start(&self) -> Result<()>;
    pub fn stop(&self) -> Result<()>;
}
```

**线程安全说明：** Engine 不是 Sync，但音频回调需要访问它。
方案：使用 `Arc<Mutex<Engine>>`，但 Mutex 只在 `start()`/`stop()` 时锁定，
音频回调中通过 `try_lock()` 获取（如果锁不到，输出静音）。
更优方案：音频线程独占 Engine，主线程通过事件队列间接操作。
**选择后者** — 音频线程独占 Engine，无 Mutex。

```
Runtime::new(engine)
  → engine 移动到 AudioThread 结构体
  → AudioThread 在 cpal 回调中独占使用 engine
  → 主线程通过 rtrb Producer 发送事件
```

### midi_input.rs — MIDI 设备输入

```rust
pub(crate) struct MidiInput {
    connection: Option<midir::MidiInputConnection<()>>,
}

impl MidiInput {
    pub fn list_devices() -> Result<Vec<MidiDeviceInfo>>;

    /// 连接 MIDI 设备。收到的 MIDI 消息转换为 AudioEvent，
    /// 通过 Producer 端写入事件队列。
    pub fn connect(
        device: &MidiDeviceInfo,
        producer: rtrb::Producer<AudioEvent>,
    ) -> Result<Self>;

    pub fn disconnect(&mut self);
}

pub struct MidiDeviceInfo {
    pub id: usize,
    pub name: String,
}
```

### sequencer.rs — MIDI 文件播放

```rust
pub(crate) struct Sequencer {
    tracks: Vec<Track>,
    tempo: f64,         // BPM
    position: u64,      // 当前 tick
    playing: bool,
    looping: bool,
    ticks_per_beat: u16,
}

/// 从 MIDI 文件加载
impl Sequencer {
    pub fn load(path: &str) -> Result<Self>;  // 用 midly crate 解析

    /// 每次音频回调调用：根据经过的 sample 数推进 tick，
    /// 产出到期的事件。
    pub fn advance(
        &mut self,
        samples: usize,
        sample_rate: u32,
        output: &mut Vec<AudioEvent>,
    );

    pub fn play(&mut self);
    pub fn pause(&mut self);
    pub fn stop(&mut self);       // 回到开头
    pub fn seek(&mut self, position: Duration);
    pub fn set_tempo(&mut self, bpm: f64);
    pub fn set_loop(&mut self, enabled: bool);
    pub fn is_playing(&self) -> bool;
    pub fn position(&self) -> Duration;
}
```

**时序精度：** Sequencer 在音频线程中运行（被 advance() 调用），
以 sample 为单位计算时间，精度 = 1/44100 秒 ≈ 22.7μs。

### transport.rs — 传输状态

```rust
/// 线程安全的传输状态，主线程写，音频线程读。
pub(crate) struct Transport {
    state: AtomicU8,      // Playing, Paused, Stopped
    tempo: AtomicU64,     // f64 bits as u64
    position: AtomicU64,  // 当前 sample 位置
    looping: AtomicBool,
}
```

用 atomics 实现主线程和音频线程之间的状态同步，无锁。

### runtime.rs — 组装

```rust
pub struct Runtime {
    producer: rtrb::Producer<AudioEvent>,
    audio_output: AudioOutput,
    midi_input: Option<MidiInput>,
    transport: Arc<Transport>,
    // Sequencer 在音频线程中，通过 Transport 控制
}
```

## 音频线程内部循环

```rust
// 在 cpal 回调中执行
fn audio_callback(
    engine: &mut Engine,
    sequencer: &mut Option<Sequencer>,
    consumer: &mut rtrb::Consumer<AudioEvent>,
    transport: &Transport,
    output: &mut [f32],  // interleaved stereo
    buffer_size: usize,
) {
    let mut left = vec![0.0f32; buffer_size];   // 预分配，不在回调中分配
    let mut right = vec![0.0f32; buffer_size];

    // 1. 排空事件队列（来自用户 API + MIDI 键盘）
    while let Ok(event) = consumer.pop() {
        dispatch_event(engine, event);
    }

    // 2. 序列器推进（产出定时事件）
    if let Some(seq) = sequencer {
        if transport.is_playing() {
            let mut seq_events = Vec::new();  // 预分配
            seq.advance(buffer_size, engine.sample_rate(), &mut seq_events);
            for event in seq_events {
                dispatch_event(engine, event);
            }
        }
    }

    // 3. 渲染
    engine.render(&mut left, &mut right);

    // 4. 交错写入 cpal 输出 buffer
    for i in 0..buffer_size {
        output[i * 2] = left[i];
        output[i * 2 + 1] = right[i];
    }
}

fn dispatch_event(engine: &mut Engine, event: AudioEvent) {
    match event {
        AudioEvent::NoteOn { channel, note, velocity } => engine.note_on(channel, note, velocity),
        AudioEvent::NoteOff { channel, note, .. } => engine.note_off(channel, note),
        AudioEvent::CC { channel, cc, value } => engine.cc(channel, cc, value),
        AudioEvent::PitchBend { channel, value } => engine.pitch_bend(channel, value),
        AudioEvent::ProgramChange { channel, program } => engine.program_change(channel, program),
        AudioEvent::AllNotesOff => engine.all_notes_off(),
        AudioEvent::SetVolume(v) => engine.set_volume(v),
        AudioEvent::Stop => engine.all_notes_off(),
    }
}
```

**注意：** 上面的 `vec![]` 在实际实现中必须是预分配的 buffer，不能在回调中分配。
Runtime 初始化时分配所有 buffer，通过闭包捕获传入回调。

## 依赖

```toml
[package]
name = "moonlitt-runtime"

[dependencies]
moonlitt-engine = { path = "../moonlitt-engine", features = ["sf2", "vst3"] }
cpal = "0.15"        # 跨平台音频 I/O（macOS CoreAudio, Win WASAPI, Linux ALSA）
midir = "0.10"       # 跨平台 MIDI I/O
midly = "0.5"        # MIDI 文件解析（纯 Rust，零依赖）
rtrb = "0.3"         # 无锁 SPSC 环形缓冲区（专为音频设计）
```

## CLI 命令更新

```bash
# 现有（用 Engine，离线渲染）
moonlitt play "Pianoteq.vst3" -o output.wav
moonlitt scan
moonlitt info "Pianoteq.vst3"
moonlitt presets "Pianoteq.vst3"

# 新增（用 Runtime，实时）
moonlitt play "Pianoteq.vst3" --live --note 60 --duration 3
moonlitt play "Pianoteq.vst3" --live --midi song.mid
moonlitt live "Pianoteq.vst3"                    # 连接 MIDI 键盘
moonlitt midi-devices                             # 列出 MIDI 设备
```

## 对 moonlitt-ffi（C API）的影响

Runtime 需要暴露为 C API 供语言绑定使用：

```c
// 生命周期
moonlitt_runtime_t* moonlitt_runtime_create(moonlitt_engine_t* engine);
void moonlitt_runtime_destroy(moonlitt_runtime_t* rt);

// 音频输出
int moonlitt_runtime_start(moonlitt_runtime_t* rt);
int moonlitt_runtime_stop(moonlitt_runtime_t* rt);

// MIDI（线程安全，无锁入队）
void moonlitt_runtime_note_on(moonlitt_runtime_t* rt, int ch, int note, int vel);
void moonlitt_runtime_note_off(moonlitt_runtime_t* rt, int ch, int note);

// MIDI 设备
char* moonlitt_runtime_list_midi_inputs(moonlitt_runtime_t* rt);
int moonlitt_runtime_connect_midi(moonlitt_runtime_t* rt, int device_id);

// 序列器
int moonlitt_runtime_load_midi(moonlitt_runtime_t* rt, const char* path);
void moonlitt_runtime_play(moonlitt_runtime_t* rt);
void moonlitt_runtime_pause(moonlitt_runtime_t* rt);
void moonlitt_runtime_seek(moonlitt_runtime_t* rt, double seconds);
```

## 对 Piano Block mod 的影响

当前 Piano Block 的架构：
```
游戏循环 Update() → AudioManager → IEngine.Render() → XNA SubmitBuffer()
```

切换到 moonlitt 后：
```
游戏循环 → moonlitt-ffi → Runtime.note_on() → 事件队列 → 音频线程 → render → cpal
```

**XNA DynamicSoundEffectInstance 不再使用** — cpal 直接驱动音频输出。
游戏只需要发送 MIDI 事件，不再管音频 buffer 提交。

## 测试策略

```
moonlitt-runtime/tests/
├── event_queue_test.rs      无锁队列：单线程正确性 + 多线程压力测试
├── audio_output_test.rs     cpal 初始化 + 短时渲染 + 验证非静音
├── midi_input_test.rs       midir 设备枚举（无需真实设备）
├── sequencer_test.rs        MIDI 文件加载 + 事件时序精度验证
│                            （合成 MIDI 文件，验证事件在正确 sample 位置触发）
├── transport_test.rs        play/pause/stop/seek 状态机测试
└── integration_test.rs      Runtime 完整流程：
                             load plugin → start → note_on → 渲染 2 秒 → 验证
```

## 开发顺序

1. event_queue.rs — 无锁队列 + 事件类型（TDD）
2. audio_output.rs — cpal 集成（TDD: 渲染到 buffer 验证）
3. runtime.rs — 组装 engine + output + queue（TDD）
4. sequencer.rs — MIDI 文件播放（TDD: 时序精度）
5. midi_input.rs — MIDI 设备输入（TDD: 设备枚举）
6. transport.rs — 传输控制 atomics（TDD: 状态机）
7. CLI 更新 — --live, --midi, midi-devices 命令
