# sushi

一个合成器项目。

## 功能

- 双振荡器：Sine / Triangle / Saw / Square / Pulse，支持八度、半音、微调、脉宽、电平
- 每个振荡器 1–4 重 Unison，带失谐与立体声展宽
- 噪声发生器、TPT 状态变量滤波器（LP/HP/BP/Notch）带 Drive
- 独立的 Amp / Filter ADSR 包络
- 双 LFO（Sine / Triangle / Saw / Square / S&H），可路由到音高、滤波、音量、脉宽
- Poly / Mono / Legato、Glide、主输出软限幅
- Chorus、Ping-Pong Delay、Reverb
- Win95 / Pascal 风格界面：全竖向推子、所有参数可精确输入数值、波形点击弹出选择、实时示波器

## 构建

```sh
cargo build --release                        # 独立程序：target/release/sushi
cargo nice-plug bundle sushi --release       # CLAP + VST3 + 独立程序
```

产物位于 `target/bundled/`，把 `sushi.clap` 放到 `~/.clap`、`sushi.vst3` 放到
`~/.vst3`，即可在 Reaper 等 DAW 中加载。

离线试听：

```sh
cargo run --release --example render_demo
pw-play target/demo.wav
```
