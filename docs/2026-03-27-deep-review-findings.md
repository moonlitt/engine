# Moonlitt 深度 Review 交接单（覆盖更新版）

日期：2026-03-27

说明：本文件覆盖同名旧版报告，以下内容以当前仓库状态为准。

目的：给其他 agent 提供一份“当前仍需修复的问题清单”，避免继续按已经过期的 findings 执行。

## 当前结论

这次覆盖更新后，可以把之前的结论分成 3 类：

1. 已修复或已缓解的问题
2. 仅修改了注释/测试策略，但核心行为问题仍在的问题
3. 继续 review 后新增确认的问题

当前最值得优先修复的，不再是“Rust E2E 会直接红”，而是 runtime 与真实音频设备的兼容性、测试误跳过、以及 FFI 边界语义。

## 当前验证状态

本轮实际执行结果：

- `cargo test --workspace`
  - 结果：通过
- `cargo clippy --workspace --all-targets`
  - 结果：通过
  - 备注：仍有若干 warning，但没有阻塞性错误

## 状态变更总览

### 已关闭 / 已修复

1. Rust runtime E2E 测试不再因为无音频设备直接让 `cargo test --workspace` 失败。
2. CLI 的 live 路径不再对 `rt.start()` 直接 `unwrap()`，设备不可用时会输出错误并退出。
3. OxiSynth 的 Reverb / Chorus 开关“关了之后开不回来”的问题已经修复，当前实现已引入缓存并在重新打开时恢复电平。
4. 原先 FFI 的 channel 越界会进入不安全路由的问题已被消除，至少不再直接把非法值送进 `1u16 << channel` 这一层。

### 部分缓解，但未完全收口

1. Rust 侧 runtime 注释已经把模型收紧成 “lock-free SPSC / single caller only”，但 `.NET` 绑定注释仍写着 “thread-safe, lock-free via ring buffer”，契约还没完全统一。
2. `moonlitt_runtime_create` 现在明确文档写出“失败也会消费 engine”，但这是行为说明，不是行为修复。
3. Rust 测试通过“失败即跳过”修掉了 CI 假红，但这个策略也把一类真实启动回归隐藏起来了。

## 当前仍然有效的 Findings

以下是按当前仓库状态整理后的有效问题，建议其他 agent 以这里为准继续修。

### 1. [P2] Runtime 输出流从不协商设备支持的配置

影响文件：

- `crates/moonlitt-runtime/src/audio_output.rs`

关键位置：

- `crates/moonlitt-runtime/src/audio_output.rs:15-27`

问题描述：

`AudioOutput::new` 当前固定请求：

- 双声道
- `f32`
- engine 自己的采样率

但没有先查询默认输出设备实际支持的格式和配置，也没有做 fallback。

这意味着：只要默认设备不支持“刚好这组参数”，`Runtime::new()` 就会失败，哪怕系统其实存在可用音频设备。

为什么这是实际产品问题：

- 这不是测试层问题，而是运行时兼容性问题。
- 当前 Rust 测试之所以不再报错，是因为测试帮助函数把这类失败当成“没有设备”直接跳过了。

建议修复方向：

- 在 `AudioOutput::new` 内查询设备支持的输出配置。
- 明确选择策略：
  - 优先匹配 engine 采样率
  - 不匹配时做显式 fallback 或返回结构化错误
- 不要把“设备存在但格式不兼容”混同为“没有设备”。

验收标准：

- `Runtime::new()` 对真实设备配置不兼容时，能给出可区分的错误原因。
- 常见 macOS 输出设备上，不因固定配置请求而无谓失败。

### 2. [P2] 新增的 skip helper 会掩盖真实的 runtime 启动回归

影响文件：

- `crates/moonlitt-runtime/tests/e2e_test.rs`
- `crates/moonlitt-runtime/tests/runtime_test.rs`

关键位置：

- `crates/moonlitt-runtime/tests/e2e_test.rs:20-33`
- `crates/moonlitt-runtime/tests/runtime_test.rs:6-20`

问题描述：

当前 `try_create_runtime()` 的逻辑是：

- `Runtime::new()` 失败 -> 直接打印 skipping 并返回 `None`
- `rt.start()` 失败 -> 直接打印 skipping 并返回 `None`

这解决了“无设备环境下测试假红”，但也把以下问题全部吞掉了：

- 设备存在但输出格式不兼容
- sample rate / channel / sample format 配置问题
- 未来 runtime 启动路径的真实回归

结果就是：

- 测试现在更绿了
- 但对 runtime 启动质量的信号也变弱了

建议修复方向：

- 只把“明确可识别的 host 环境缺失”当作 skip。
- 把“设备存在但启动失败”视为真正失败。
- 最好把错误分类做出来，而不是字符串层面笼统吞掉。

验收标准：

- 无设备环境依然可稳定跳过。
- 配置不兼容或启动回归不能再变成假绿。

### 3. [P2] Runtime 创建失败仍会消费 engine，当前只是把行为写进了文档

影响文件：

- `crates/moonlitt-ffi/src/runtime_api.rs`

关键位置：

- `crates/moonlitt-ffi/src/runtime_api.rs:25-30`
- `crates/moonlitt-ffi/src/runtime_api.rs:48-64`

问题描述：

当前实现依旧是在 `moonlitt_runtime_create()` 里先把 `EngineHandle` 里的 engine `take()` 出来，再尝试创建 `Runtime`。

如果失败：

- 返回空指针
- 但 engine 已被消费，调用者不能复用它重试

现在代码做的改变主要是：

- 把这个语义写进了注释

但行为本身没有修。

为什么仍然值得修：

- 这是一个很不友好的 FFI 契约。
- 调用方必须额外重建 engine，成本和语义都比较差。
- 当失败原因只是音频设备暂时不可用时，这种“失败即销毁 engine”的行为尤其不自然。

建议修复方向：

- 成功创建 runtime 之后再转移所有权。
- 或在失败路径里把 engine 放回原 handle。

验收标准：

- runtime 创建失败后，调用方仍能对同一个 engine handle 做明确且合理的后续操作。

### 4. [P2] delayed event 突发场景仍会在实时线程上触发额外开销

影响文件：

- `crates/moonlitt-runtime/src/audio_thread.rs`

问题描述：

之前指出的 delayed event 问题目前看没有实质修复：

- `pending` 仍然是 `Vec`
- 仍有容量上界之外的扩容风险
- 仍在 callback 内执行 `sort_by_key`
- 仍会做前段 `drain`

这与 runtime / mixer 对 realtime 场景的目标并不一致。

建议修复方向：

- 明确 delayed event 的容量模型
- 尽量避免 callback 内扩容
- 重新评估排序与移除策略，保证实时线程开销上界可控

验收标准：

- 压力场景下 callback 不再因 delayed event 突发而退化到不可预测的分配 / 排序成本。

### 5. [P3] FFI 现在通过 clamp 静默改写非法 MIDI 输入，而不是拒绝它

影响文件：

- `crates/moonlitt-ffi/src/runtime_api.rs`
- `crates/moonlitt-ffi/src/engine_api.rs`

关键位置：

- `crates/moonlitt-ffi/src/runtime_api.rs:116-118`
- `crates/moonlitt-ffi/src/engine_api.rs:131-178`

问题描述：

当前新的边界处理策略是把非法值直接截到合法范围，例如：

- `channel=99` -> `15`
- `note=-1` -> `0`
- `velocity=999` -> `127`

这样确实避免了越界和 UB，但也会把调用方 bug 静默变成另一条真实 MIDI 消息。

风险：

- 调用方更难定位问题
- 线上行为会变成“发出了错误但合法的音”
- 对公共 FFI 来说，silent coercion 通常比 reject/no-op 更难排查

建议修复方向：

- 对非法输入优先考虑：
  - 返回错误
  - 或安全 no-op
- 不建议把明显非法输入静默改造成别的合法事件

验收标准：

- 非法输入不会再被自动改写成不同的音乐语义。

### 6. [P3] runtime 并发契约仍未在所有绑定层统一

影响文件：

- `crates/moonlitt-runtime/src/runtime.rs`
- `crates/moonlitt-ffi/src/runtime_api.rs`
- `bindings/dotnet/src/Moonlitt/Runtime.cs`
- `bindings/dotnet/src/Moonlitt/NativeApi.cs`

关键位置：

- `crates/moonlitt-runtime/src/runtime.rs:80`
- `crates/moonlitt-ffi/src/runtime_api.rs:108-111`
- `bindings/dotnet/src/Moonlitt/Runtime.cs:57-59`
- `bindings/dotnet/src/Moonlitt/NativeApi.cs:100-102`

问题描述：

Rust 侧现在已经把 runtime 事件发送模型写成：

- lock-free
- SPSC
- single caller only

但 `.NET` 层注释仍然写着：

- thread-safe
- lock-free

这会让高层调用者继续误以为可以安全并发调用同一个 runtime handle。

建议修复方向：

- 把所有公开层的注释、README、绑定文档统一成同一套契约。
- 如果长期目标仍是 thread-safe，就需要真正改实现；如果不是，就不要在绑定层继续宣传 thread-safe。

验收标准：

- 所有语言绑定对 runtime 并发模型的描述一致。

## 建议给其他 agent 的任务拆分

### Agent A：Runtime 设备协商与错误分类

负责：

- `crates/moonlitt-runtime/src/audio_output.rs`
- 相关 runtime 错误传播路径

目标：

- 修复输出配置协商缺失
- 区分“无设备”和“配置不兼容/启动失败”

### Agent B：测试策略修正

负责：

- `crates/moonlitt-runtime/tests/e2e_test.rs`
- `crates/moonlitt-runtime/tests/runtime_test.rs`

目标：

- 保留无设备环境 skip
- 防止真实启动回归被 skip helper 吞掉

### Agent C：FFI 边界语义

负责：

- `crates/moonlitt-ffi/src/runtime_api.rs`
- `crates/moonlitt-ffi/src/engine_api.rs`
- `.NET` 绑定注释/说明

目标：

- 重新定义非法 MIDI 输入处理策略
- 统一 runtime 的并发契约
- 评估 runtime 创建失败时的 engine 所有权语义

### Agent D：Realtime 路径优化

负责：

- `crates/moonlitt-runtime/src/audio_thread.rs`

目标：

- 继续处理 delayed event 在音频线程上的扩容/排序问题

## 修复完成后建议重新执行

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets`

如果改动了 `.NET` 绑定，也建议补跑其对应测试项目。
