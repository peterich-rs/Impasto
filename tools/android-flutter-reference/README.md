# Android Flutter 行情参考

Flutter 参考应用用 Gallery 的 `QuoteEngine` 生成相同标的及 200 ms 间隔的确定性行情快照。它提供列表及逐行走势图、腾讯控股详情、分时、日 K、盘口和逐笔成交。应用能暂停行情，支持拖动图表光标和纵向滚动。行情仿真在构建时完成，测量可用于定位布局、绘制和呈现的差异。两端的整帧 CPU 耗时还受各自运行时及 Gallery 实时仿真的影响。

在仓库根目录准备 Flutter SDK、JDK 17、Android SDK 和支持 TypeScript 类型擦除的 Node。构建脚本使用 `FLUTTER` 指定的工具。Flutter 工程、行情数据和 `flutter-version.json` 生成到 `target/android-flutter-reference`。

```sh
FLUTTER=/path/to/flutter bash tools/android-flutter-reference/build.sh
adb install -r target/android-flutter-reference/build/app/outputs/flutter-apk/app-profile.apk
adb shell am start -W -n dev.impasto.market_reference/.MainActivity
```

比较时保持同一设备、分辨率和刷新率。和 Gallery 一样，回放对列表及详情执行往返纵向滑动，对图表执行水平拖动。使用 `ANDROID_SERIAL` 指定设备。脚本保存系统 trace、SurfaceFlinger 呈现时间和设备配置。Flutter trace 使用该 Flutter 版本提供的区间名称，可在 Perfetto 中查看这个包及其 SurfaceView 的帧时间线。

```sh
FRAME_REFERENCE=flutter FRAME_REFRESH=60 tools/android-frame-replay.sh target/flutter-reference/60 3
FRAME_REFERENCE=flutter FRAME_REFRESH=120 tools/android-frame-replay.sh target/flutter-reference/120 3
python3 tools/android-frame-report.py target/flutter-reference/60 --hz 60 --reference flutter
python3 tools/android-frame-report.py target/flutter-reference/120 --hz 120 --reference flutter
```
