# sushi

一个合成器项目。

## 功能

**声音**

- 双振荡器：Sine / Triangle / Saw / Square / Pulse，支持八度、半音、微调、脉宽、电平
- 每个振荡器 1–4 重 Unison，带失谐与立体声展宽
- 噪声、TPT 状态变量滤波器（LP/HP/BP/Notch）带 Drive
- 独立的 Amp / Filter ADSR 包络
- 双 LFO（Sine / Triangle / Saw / Square / S&H），可路由到音高、滤波、音量、脉宽
- 每声部模拟漂移（Drift），Poly / Mono / Legato、Glide
- **Character**：Silk / Grit 两档性格，统一改变饱和、驱动和漂移
- **宏控制**：Brilliance / Motion / Space，每个同时带动一组参数
- 力度同时影响音量与滤波器

**可玩性**

- **琶音器**：Up / Down / UpDown / Random，1/4–1/32，1–3 八度，Gate、Latch，跟随宿主速度
- **8 步调制音序器**：可指向 Pitch / Filter / Shape，支持平滑
- **8 个工厂预设** + **MUTATE** 随机演化
- 小示波器、ACT / CLIP / 声部 LED、步进 LED

**界面**

- Win95 / Pascal 风格单页面板：竖向推子（滑块完全嵌入凹槽）、所有参数可精确输入数值、波形/模式点击弹出
- 4 行模块化分区：OSC / FILTER / MIX · ENV / LFO · STEP / ARP / VOICE · FX / MACROS / OUTPUT

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
