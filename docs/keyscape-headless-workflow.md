# Keyscape（及同类采样流送插件）Headless 工作流

商业采样器（Spectrasonics Keyscape / Omnisphere、Kontakt 类）的音色库是
私有格式：预设列表不通过 VST3 标准接口暴露，headless 直接加载只会得到
**静音**（这是插件的设计，不是宿主的 bug）。moonlitt 的解法是
**捕捉一次、永久重放**：

```
┌─────────────┐  Save State   ┌──────────────┐  load_state    ┌──────────┐
│ 桌面 app     │ ────────────▶ │ .mlstate 文件 │ ─────────────▶ │ 游戏/CLI  │
│（插件 GUI）  │               │（MLST 容器）  │   + warm_up    │ headless │
└─────────────┘               └──────────────┘                └──────────┘
       一次性的、需要 GUI                                       永久的、无 GUI
```

## 第 1 步 — 捕捉音色（一次性，需要 GUI）

打开 moonlitt 桌面 app，加载 Keyscape，在插件自己的界面里选好音色
（例如 "LA Custom C7"），然后 **Save State**。得到一个 `.mlstate` 文件
—— MLST 容器（magic + version + IComponent/IEditController 两段状态），
约 250 KB。

任何能宿主 VST3 并导出插件状态的 DAW 同样可用。

## 第 2 步 — Headless 重放（每次，无 GUI）

### C API（游戏 mod 的路径）

```c
EngineHandle *e = moonlitt_engine_create(44100, 256);
moonlitt_engine_load(e, "/Library/Audio/Plug-Ins/VST3/Keyscape.vst3");

/* 恢复捕捉的音色 */
moonlitt_engine_load_state(e, blob, blob_len);

/* 关键：采样流送器异步加载样本 —— 必须先泵 warm-up，
 * 否则接下来渲染的全是静音。 */
int blocks = moonlitt_engine_recommended_warmup_blocks(e);  /* Spectrasonics → 非 0 */
moonlitt_engine_warm_up(e, blocks);

/* 现在可以出声了 */
moonlitt_engine_note_on(e, 0, 60, 100);
moonlitt_engine_render(e, left, right, 256);
```

实时播放用 `moonlitt_runtime_*` 家族；`.mlsession` 会话文件则把
state + warm-up 打包在一起，`moonlitt_session_load_from_file` 一步还原
（恢复时自动 warm-up）。

### CLI

```bash
moonlitt midi song.mid --sound Keyscape.vst3 --state la-custom-c7.mlstate -o out.wav
```

`--state` 之后 CLI 自动执行推荐 warm-up（离线渲染与实时模式都覆盖）。

## 为什么需要 warm-up

`load_state` 返回后，流送器在后台线程继续把样本拉进内存。这个窗口里
触发的音符会被插件静默丢弃。`warm_up(n)` 让引擎先空转 n 个处理块
（Spectrasonics 推荐 8192 块 ≈ 47 秒音频时长，墙钟时间通常 1–3 秒，
取决于磁盘缓存），等流送管线就绪。

`recommended_warm_up_blocks()` 按厂商自动识别；对非流送后端恒为 0，
`warm_up` 是无害的 no-op —— 所以"加载 state 后一律 warm-up"是安全的
通用写法。

## 回归防线

- `moonlitt-capi/tests/ffi_test.rs::test_keyscape_headless_replay_through_c_api`
  —— C ABI 全链路（fixture 门控，机器上没有 Keyscape 时自动跳过）
- `moonlitt-cli/tests/keyscape_warmup_test.rs` —— CLI 离线渲染回归
  （曾因漏 warm-up 渲染出整文件静音）
- `moonlitt-engine/tests/vst3_shared_handle.rs` —— GUI 侧 set_state 与
  音频后端共享同一插件实例
- fixture: `crates/moonlitt-vst3/tests/fixtures/keyscape-default.mlstate`
