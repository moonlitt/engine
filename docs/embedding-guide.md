# moonlitt 嵌入指南（C ABI）

moonlitt 是纯 Rust 的 headless DAW 引擎：宿主 VST3/CLAP 插件、合成 SF2，
带多轨混音、内置效果器和会话持久化。本指南面向想把它嵌进游戏 mod /
任何原生宿主的开发者。

## 拿到三样东西

1. **`libmoonlitt.{dylib,so,dll}`** — `cargo build -p moonlitt-capi --release`
   产物在 `target/release/`。
2. **`include/moonlitt.h`** — 完整 C 头文件（cbindgen 生成，CI 保证与源码
   零漂移）。每个符号带文档：参数语义、单位、取值范围、所有权、线程契约。
3. **参考绑定** — C# P/Invoke 声明在
   `examples/ffi-testbed-csharp/NativeEngine.cs`（游戏 mod 可逐字拷贝）；
   更高层的 .NET 封装在 `bindings/dotnet/`。

## 五条铁律（来自 ABI 0.9 约定）

1. **可失败函数返回 `MoonlittStatus`**：`0` 成功，负数是错误类
   （`MOONLITT_ERR_INVALID_ARG` / `NOT_LOADED` / `QUEUE_FULL` / `IO` /
   `PLUGIN` / `STATE` / `PANIC` / `UNSUPPORTED`）。没有静默失败。
2. **错误详情**：`moonlitt_last_error_message()`（调用线程局部，借用指针
   **不要 free**，仅在拿到非 0 状态后读取才有意义）。
3. **所有权**：文档标注 owned 的字符串/缓冲区必须用
   `moonlitt_free_string` / `moonlitt_free_buffer` 释放；borrowed 的禁止
   free。`add_track` 一族会**消耗** EngineHandle 里的 backend（空壳仍需
   destroy，二次使用得到 `NOT_LOADED`）。
4. **越界即拒绝**：参数超范围返回 `INVALID_ARG`，绝不静默截断。
5. **单控制线程**：每个 handle 的所有调用来自同一线程（通常是游戏主
   线程）；音频线程是库内部的，与你无关。事件函数是 wait-free 的，队列
   满返回 `QUEUE_FULL`（丢弃，下帧重试）。

加载时校验 ABI：`moonlitt_abi_version()` 返回 `(major<<16)|(minor<<8)|patch`。
绑定可调 `moonlitt_debug_trigger_panic()` 自检 panic 防线（必须得到
`MOONLITT_ERR_PANIC` 而非进程崩溃）。

## 最小可发声（C 伪码）

```c
#include "moonlitt.h"

EngineHandle *e = moonlitt_engine_create(44100, 256);
if (moonlitt_engine_load(e, "GeneralUser_GS.sf2") != MOONLITT_OK) {
    log("load failed: %s", moonlitt_last_error_message());
}

/* 实时出声：runtime 接管音频设备 */
RuntimeHandle *rt = moonlitt_runtime_create(e);   /* backend 被消耗 */
moonlitt_runtime_start_audio(rt);
moonlitt_runtime_note_on(rt, /*ch*/0, /*note*/60, /*vel*/100);
/* … */
moonlitt_runtime_note_off(rt, 0, 60);

moonlitt_runtime_destroy(rt);
moonlitt_engine_destroy(e);   /* 空壳也要销毁 */
```

离线渲染（不占音频设备）：跳过 runtime，直接
`moonlitt_engine_render(e, left, right, frames)`。

## 常用台阶

| 需求 | 入口 |
|------|------|
| 一行起一个 16 轨 SF2 引擎（按通道分轨） | `moonlitt_runtime_create_multitrack_sf2(sf2, 44100, 256)` |
| 商业采样器音色（Keyscape 等） | `moonlitt_engine_load_state` + `moonlitt_engine_warm_up`，见 [keyscape-headless-workflow.md](keyscape-headless-workflow.md) |
| 整套工程一步还原 | `.mlsession` 文件 + `moonlitt_session_load_from_file(path, 256)`（自动 warm-up）；启动前 `moonlitt_session_validate_file` 深度预检 |
| 保存当前工程 | `moonlitt_runtime_save_session(rt, path)` |
| 采样精确时序（节奏游戏） | `moonlitt_runtime_note_on_delayed(rt, ch, note, vel, delay_samples)` |
| 电平表 / 运行状态 | `moonlitt_runtime_master_peak` / `master_rms` / `is_running`（原子读，可轮询） |
| 内置效果器（19 种） | `moonlitt_builtin_create_*` → `moonlitt_runtime_add_insert` / `add_send_bus` |

## 验证你的绑定

`examples/ffi-testbed-csharp/run.sh` 跑 100+ 项 ABI 一致性检查（每个调用
断言状态码、panic 防线穿透验证、所有权往返）。改了绑定先跑它——
通过则问题不在 FFI 层。
