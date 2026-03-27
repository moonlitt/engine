# Moonlitt 深度 Review 交接单

日期：2026-03-27

目的：把这次仓库 review 的核心问题整理成可交给其他 agent 修复的工作单。

状态：仅整理问题与建议，当前未在本文件对应的问题上直接提交修复代码。

## 本次检查范围

- Rust workspace 的主要 crate：
  - `moonlitt-runtime`
  - `moonlitt-engine`
  - `moonlitt-ffi`
  - `moonlitt-cli`
- `.NET` 绑定：
  - `bindings/dotnet`
- 验证动作：
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets`

## 总结

这次 review 里最需要优先处理的不是样式或小型 lint，而是 3 类更实质的问题：

1. FFI 对外宣称线程安全，但当前实现并不满足这个承诺，存在未定义行为风险。
2. runtime / CLI / 测试对系统音频设备的假设过强，导致 workspace 测试不稳定，也会让实际 CLI 在无设备或设备异常环境下直接崩掉。
3. 实时音频线程的 delayed event 路径仍可能分配内存，与“无分配”目标冲突。

## 建议修复顺序

1. 先修 FFI 线程安全问题
2. 再修 runtime 创建失败时的所有权语义
3. 再修 channel / 参数等外部输入校验
4. 再修 delayed event 的实时线程分配问题
5. 再修 SF2 效果开关语义
6. 最后修 E2E / CLI 的设备依赖与测试策略

## Findings

### 1. [P1] FFI runtime 方法并不真正线程安全

影响文件：

- `crates/moonlitt-ffi/src/runtime_api.rs`
- `bindings/dotnet/src/Moonlitt/Runtime.cs`
- `bindings/dotnet/src/Moonlitt/NativeApi.cs`

关键位置：

- `crates/moonlitt-ffi/src/runtime_api.rs:99`

问题描述：

`runtime_api.rs` 中多个 `extern "C"` 导出函数被注释描述为“thread-safe, lock-free”，但实现是把裸指针直接通过 `rt.as_mut()` 转成 `&mut Runtime`。如果外部语言从两个线程同时调用同一个 runtime handle，就会在 Rust 侧制造同一对象的并发可变借用，这是未定义行为。

为什么严重：

- 这是 ABI / 内存模型层面的错误，不是普通逻辑 bug。
- `.NET` 层当前也没有序列化这些调用，因此问题对真实调用方可见。

建议修复方向：

- 不要再把共享 FFI handle 直接映射成 `&mut Runtime`。
- 需要明确 runtime FFI 的并发模型，二选一：
  - 方案 A：把 `RuntimeHandle` 设计成内部自带同步，允许并发调用。
  - 方案 B：文档和 API 都明确声明“不可并发调用”，并在绑定层提供串行化保护。
- 如果要保留“thread-safe”承诺，建议把可并发入口设计为只操作线程安全的内部组件，而不是对整个 `Runtime` 取独占可变引用。

验收标准：

- 同一 runtime handle 的并发 FFI 调用不再依赖 UB。
- 文档、Rust FFI 实现、.NET 包装层三者的线程模型一致。
- 至少增加一个并发调用测试，覆盖重复 `note_on` / `set_volume` / `cc` 等路径。

### 2. [P2] runtime 创建失败时，会提前吃掉 engine 所有权

影响文件：

- `crates/moonlitt-ffi/src/runtime_api.rs`
- `bindings/dotnet/src/Moonlitt/Runtime.cs`

关键位置：

- `crates/moonlitt-ffi/src/runtime_api.rs:38`

问题描述：

`moonlitt_runtime_create` 先从 `EngineHandle` 里 `take()` 出 engine，再调用 `Runtime::new(engine)`。如果 `Runtime::new` 因系统音频设备不可用而失败，函数会返回空指针，但原 `EngineHandle` 里的 engine 已经被移走，调用方无法继续使用这个 engine 重试。

后果：

- FFI 调用方会得到“创建 runtime 失败”，但 engine 其实已经被消费。
- 高层绑定可能仍保留一个看似有效的 `Engine` 对象，语义上非常混乱。

建议修复方向：

- 只有在 runtime 真正构造成功后，才从 `EngineHandle` 里移交所有权。
- 或者在失败路径里把 engine 放回原 handle。
- 高层绑定应把“构造失败时 engine 是否仍可继续使用”定义清楚并保持一致。

验收标准：

- 当 runtime 创建失败时，调用方能安全重试，或能得到明确定义的失败语义。
- 为此增加单测，覆盖音频设备不可用或 mock 失败场景。

### 3. [P2] 外部输入没有做 channel 边界校验

影响文件：

- `crates/moonlitt-ffi/src/engine_api.rs`
- `crates/moonlitt-ffi/src/runtime_api.rs`
- `crates/moonlitt-runtime/src/mixer.rs`

关键位置：

- `crates/moonlitt-runtime/src/mixer.rs:155`

问题描述：

FFI 层把外部传入的 `c_int` 直接 `as u8`，而 mixer 内部再做 `1u16 << channel` 来路由 MIDI 通道。这样一来，负数或超范围输入不会被拒绝，而是会被截断成错误通道值，甚至进入非法移位路径。

后果：

- C / C# / 其他语言调用者只要给错 channel，就可能得到难以诊断的行为。
- 这类 bug 会表现成“音符发不出去”“发错轨道”“偶发异常”，排查成本高。

建议修复方向：

- 在 FFI 边界先验证：
  - `channel` 必须在 `0..=15`
  - `note` / `velocity` / `cc` / `program` 要按协议范围校验
  - `pitch_bend` 要做合理范围约束
- mixer 内部也建议保底防御，不要假设所有上游都已校验。

验收标准：

- 非法输入要么返回错误，要么安全 no-op，但不能静默截断成错误值。
- 增加 FFI 层测试，覆盖负数、超范围 channel 和 note/control 输入。

### 4. [P2] delayed event 突发场景会在实时线程上分配内存

影响文件：

- `crates/moonlitt-runtime/src/audio_thread.rs`
- `crates/moonlitt-runtime/src/runtime.rs`

关键位置：

- `crates/moonlitt-runtime/src/audio_thread.rs:61`
- `crates/moonlitt-runtime/src/audio_thread.rs:112`
- `crates/moonlitt-runtime/src/audio_thread.rs:143`

问题描述：

`AudioThread` 里的 `pending` 只预留了 128 个 delayed events，但上游 ring buffer 容量是 1024。高峰时 delayed event 一多，就会在音频回调线程里触发 `Vec` 扩容。后续还会做 `sort_by_key` 和前段 `drain`，这些都不适合 realtime audio callback。

为什么重要：

- 这和模块注释里的“no allocations”目标冲突。
- 在真实高密度 MIDI / automation / sample-accurate scheduling 场景里，有音频爆音风险。

建议修复方向：

- 明确 delayed event 的最大容量策略。
- 倾向于使用固定容量结构或可预测的预分配策略。
- 避免 callback 内部的可变长度扩容。
- 如果仍保留排序，至少要证明其上界与实时性预算可接受。

验收标准：

- 音频回调线程在 delayed event 压力场景下不再扩容分配。
- 增加压力测试，验证 delayed events 数量接近 ring buffer 上限时不会触发实时线程退化。

### 5. [P2] SF2 的 Reverb / Chorus 开关会出现“开不回来”的语义错误

影响文件：

- `crates/moonlitt-engine/src/backends/oxisynth.rs`

关键位置：

- `crates/moonlitt-engine/src/backends/oxisynth.rs:198`

问题描述：

当前关闭 Reverb / Chorus 时是直接把 level 写成 0；但重新打开时并没有恢复之前的非零 level。这样用户执行一次 off -> on 之后，参数显示是开着，但声音效果仍然等价于关闭。

后果：

- 参数系统对外语义不一致。
- 自动化、宿主 UI、绑定层都会被这个状态欺骗。

建议修复方向：

- 开关应与具体 level 分离：
  - 要么缓存关闭前的 level，重新打开时恢复。
  - 要么改成真正的 on/off 语义，而不是借助把 level 归零来模拟。
- `get_param()` 返回值要和实际 DSP 状态一致。

验收标准：

- Off -> On round-trip 后，效果器能恢复到预期可听状态。
- 为 Reverb 和 Chorus 各补一组参数回归测试。

### 6. [P2] runtime 的 E2E 测试并不 CI-safe

影响文件：

- `crates/moonlitt-runtime/tests/e2e_test.rs`
- `crates/moonlitt-runtime/tests/runtime_test.rs`
- `bindings/dotnet/tests/Moonlitt.Tests/RuntimeTests.cs`
- `crates/moonlitt-cli/src/main.rs`

关键位置：

- `crates/moonlitt-runtime/tests/e2e_test.rs:18`
- `crates/moonlitt-cli/src/main.rs:237`
- `crates/moonlitt-cli/src/main.rs:267`
- `crates/moonlitt-cli/src/main.rs:461`

问题描述：

E2E 测试虽然写了“插件不存在时跳过”，但没有把“无音频设备 / 默认输出设备不可用”视为可跳过条件。因此 `cargo test --workspace` 在当前环境下直接失败。我实际运行时，4 个 runtime E2E 测试都因为：

`The requested device is no longer available. For example, it has been unplugged.`

而失败。

此外，CLI 的实时播放路径也直接 `rt.start().unwrap()`，意味着这个问题不仅影响测试，也会让真实命令行用户在无设备环境下直接 panic。

建议修复方向：

- 把“插件存在但音频设备不可用”纳入可跳过的 host dependency 条件。
- 把 runtime 的核心逻辑测试和真正依赖系统设备的 E2E 测试分层。
- CLI 的 live 路径要改成用户可读错误，而不是 `unwrap()` panic。

验收标准：

- `cargo test --workspace` 在无可用音频设备环境中应保持稳定，不因这类 host 条件直接失败。
- CLI 在设备不可用时输出明确错误信息并非零退出，而不是 panic。

## 实际验证结果

### 1. `cargo test --workspace`

结果：失败

失败集中在：

- `crates/moonlitt-runtime/tests/e2e_test.rs`
  - `e2e_pianoteq_runtime`
  - `e2e_sf2_polyphony_stress`
  - `e2e_transport_controls`
  - `e2e_volume_control`

失败原因摘要：

- 默认输出设备不可用
- 当前测试把这类环境问题当成普通 `unwrap()` 失败处理

### 2. `cargo clippy --workspace --all-targets`

结果：失败

当前看到的是一个次要问题：

- `crates/moonlitt-runtime/src/mixer.rs` 测试里使用了近似常量 `0.7071`
- clippy 建议改用标准常量

说明：

- 这个 lint 问题不属于本次最重要的修复项
- 但如果有人顺手修，建议一起带上

## 建议拆分给其他 agent 的方式

### Agent A：FFI / 绑定安全性

建议负责：

- `crates/moonlitt-ffi`
- `bindings/dotnet`

目标：

- 修复 runtime FFI 线程安全承诺与实现不一致的问题
- 修复 runtime 创建失败时 engine 所有权被提前消费的问题
- 增加对应测试

### Agent B：runtime 实时性与输入校验

建议负责：

- `crates/moonlitt-runtime`
- `crates/moonlitt-cli`

目标：

- 修复 delayed event 在音频线程上的分配问题
- 修复 channel / MIDI 外部输入校验
- 修复 live CLI 的 `unwrap()` 崩溃

### Agent C：engine 参数语义与回归测试

建议负责：

- `crates/moonlitt-engine`

目标：

- 修复 OxiSynth backend 中 Reverb / Chorus 开关语义
- 增加参数 round-trip 回归测试

### Agent D：测试策略和环境隔离

建议负责：

- `crates/moonlitt-runtime/tests`
- `bindings/dotnet/tests`

目标：

- 调整 E2E / integration / host-dependent 测试边界
- 确保 workspace 测试在无音频设备环境也可稳定运行

## 修复后建议重新执行

建议修复完成后至少重新跑：

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets`

如果修改了 `.NET` 绑定，也建议补跑对应测试项目。
