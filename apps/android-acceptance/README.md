# Android 现场记录验收应用

这个应用从 `impasto create android-acceptance` 生成。它声明 `impasto-demo://record/<编号>` 深链，支持从原生文档选择器导入文件、请求相机权限及通过 Sharesheet 分享文本。路由、滚动位置和小型表单状态使用框架的任务快照，手动保存的草稿使用 `AsyncStorage`。系统重建进程后先恢复任务，再处理新交付的深链。

在本目录运行 `impasto analyze` 可检查 TSX。构建需要 JDK 17、Android SDK 35、NDK 28.2.13676358 和 `aarch64-linux-android`、`x86_64-linux-android` 两个 Rust 目标。`[android].abis` 可选择要打包的 ABI。

```sh
impasto build apk --debug
adb install -r build/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -W -a android.intent.action.VIEW \
  -d impasto-demo://record/42 dev.impasto.fieldnotes
adb push assets/inspection.txt /sdcard/Download/impasto-inspection.txt
```

输入检查备注，选择 Downloads 中的 `impasto-inspection.txt`。要验证任务快照，可以直接退回系统桌面。要验证持久化，可以先点击保存草稿。等应用进入后台再运行下面的命令。`pidof` 无输出表示旧进程已退出。重新打开同一个任务后应显示记录 42、备注和附件名。

```sh
adb shell input keyevent KEYCODE_HOME
adb shell am kill dev.impasto.fieldnotes
adb shell pidof dev.impasto.fieldnotes
adb shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -f 0x10200000 \
  -n dev.impasto.fieldnotes/dev.impasto.host.ImpastoActivity
```

`impasto build apk`、`impasto build appbundle` 和 `impasto build aar` 构建发布变体。APK 和 AAB 可用环境变量 `IMPASTO_ANDROID_KEYSTORE`、`IMPASTO_ANDROID_STORE_PASSWORD`、`IMPASTO_ANDROID_KEY_ALIAS`、`IMPASTO_ANDROID_KEY_PASSWORD` 指定同一签名配置。无签名配置时，APK 文件名为 `app-release-unsigned.apk`，AAB 也是未签名产物。JS 源映射位于 `build/app.js.map`，APK 发布构建的原生符号位于 `build/android/app/build/outputs/native-debug-symbols/release/`。

使用 `tools/check-android-package.py` 检查发布归档的 ABI 和 ELF LOAD 段。APK 发布变体还检查未压缩 JNI 库的 ZIP 对齐。连接 16 KB 设备后用 `adb shell getconf PAGE_SIZE` 确认返回 `16384`，再安装并运行同一流程。[Android 16 KB 设备验证](https://developer.android.com/guide/practices/page-sizes)

## 原生 Fragment

先运行 `impasto build aar`。`native-host/` 是独立 Android 项目，接收 `build/android/host/build/outputs/aar/host-release.aar`，并显式声明本地 AAR 的 AndroidX 依赖。应用在原生 Activity 中显示标题，将同一份页面放入 Fragment。Fragment 用 `viewLifecycleOwner` 挂载宿主，并传入稳定的 `restorationId` 和宿主的返回派发器。销毁 View 时关闭会话。文件选择走原生 Activity 的 Result Registry。页面中的原生补充说明和清单分别由 `EditText` 和 `ScrollView` 提供。点击原生工具栏的「拍摄现场照片」打开系统相机，返回的 Bitmap 经 RGBA 生产者 Surface 交给页面中的媒体纹理。原生宿主拥有自己的包名、权限声明和应用数据。生成的独立项目通过 `[android].permissions` 为 APK 或 AAB 写入 Android 权限声明。

```sh
../../crates/embedder/android/apk/gradlew -p native-host assembleDebug
adb install -r native-host/build/outputs/apk/debug/native-host-debug.apk
adb shell am start -W -n dev.impasto.fieldnotes.embedded/.NativeActivity
```
