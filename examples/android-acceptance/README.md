# 现场记录验收应用

这个应用从 `impasto create android-acceptance` 生成。它声明 `impasto-demo://record/<编号>` 深链，支持从原生文档选择器导入文件、请求相机权限及通过系统分享面板发送备注和附件。记录页也提供应用内浏览器、网络状态、应用信息、触感反馈及剪贴板格式探测。路由、滚动位置和小型表单状态使用框架的任务快照，手动保存的草稿使用 `AsyncStorage`。系统重建进程后先恢复任务，再处理新交付的深链。

在本目录运行 `impasto analyze` 可检查 TSX。构建需要 JDK 17、Android SDK 35、NDK 28.2.13676358 和 `aarch64-linux-android`、`x86_64-linux-android` 两个 Rust 目标。`[android].abis` 可选择要打包的 ABI。

```sh
impasto build apk --debug
adb install -r build/android/gradle/app/outputs/apk/debug/app-debug.apk
adb shell am start -W -a android.intent.action.VIEW \
  -d impasto-demo://record/42 dev.impasto.fieldnotes
adb push assets/inspection.txt /sdcard/Download/impasto-inspection.txt
```

输入检查备注，选择 Downloads 中的 `impasto-inspection.txt`。要验证任务快照，可以直接退回系统桌面。要验证持久化，可以先点击保存草稿。等应用进入后台再运行下面的命令。`pidof` 无输出表示旧进程已退出。重新打开同一个任务后应显示记录 42、备注和附件名。

在记录页分别请求相机、相册、定位和精确闹钟权限。Android 14 及以上选择部分照片后应显示 `limited`，定位只选大致位置时也应显示 `limited`。Android 12 及以上允许精确闹钟后从设置返回，结果应自动更新。打开权限弹窗后旋转屏幕，确认 Activity 重建后原请求仍返回结果。

```sh
adb shell input keyevent KEYCODE_HOME
adb shell am kill dev.impasto.fieldnotes
adb shell pidof dev.impasto.fieldnotes
adb shell am start -W -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -f 0x10200000 \
  -n dev.impasto.fieldnotes/dev.impasto.host.ImpastoActivity
```

`impasto build apk`、`impasto build appbundle` 和 `impasto build aar` 构建发布变体。APK 和 AAB 可用环境变量 `IMPASTO_ANDROID_KEYSTORE`、`IMPASTO_ANDROID_STORE_PASSWORD`、`IMPASTO_ANDROID_KEY_ALIAS`、`IMPASTO_ANDROID_KEY_PASSWORD` 指定同一签名配置。无签名配置时，APK 文件名为 `app-release-unsigned.apk`，AAB 也是未签名产物。JS 源映射位于 `build/android/bundle/release/app.js.map`，APK 发布构建的原生符号位于 `build/android/gradle/app/outputs/native-debug-symbols/release/`。

使用 `tools/check-android-package.py` 检查发布归档的 ABI 和 ELF LOAD 段。APK 发布变体还检查未压缩 JNI 库的 ZIP 对齐。连接 16 KB 设备后用 `adb shell getconf PAGE_SIZE` 确认返回 `16384`，再安装并运行同一流程。[Android 16 KB 设备验证](https://developer.android.com/guide/practices/page-sizes)

## 原生 Fragment

先运行 `impasto build aar`。`native-host/` 是独立 Android 项目，接收 `build/android/aar/app-release.aar` 及同目录的插件 AAR，显式声明 AndroidX 依赖，并为嵌入的窗口注册插件。应用在原生 Activity 中显示标题，将同一份页面放入 Fragment。Fragment 用 `viewLifecycleOwner` 挂载宿主，并传入稳定的 `restorationId` 和宿主的返回派发器。销毁 View 时关闭会话。文件选择走原生 Activity 的 Result Registry。页面中的原生补充说明和清单分别由 `EditText` 和 `ScrollView` 提供。点击原生工具栏的「拍摄现场照片」打开系统相机，返回的 Bitmap 经 RGBA 生产者 Surface 交给页面中的媒体纹理。原生宿主拥有自己的包名、权限声明和应用数据。生成的独立项目通过 `[android].permissions` 为 APK 或 AAB 写入 Android 权限声明。

```sh
../../crates/embedder/android/apk/gradlew -p native-host assembleDebug
adb install -r native-host/build/outputs/apk/debug/native-host-debug.apk
adb shell am start -W -n dev.impasto.fieldnotes.embedded/.NativeActivity
```

## iOS 模拟器与真机

同一个 TSX 入口在 iOS 用于验收深链、Scene 恢复、输入法、无障碍、系统文件选择、授权和分享。生成的 iOS 包使用独立的应用容器。

在本目录构建并安装模拟器包。若多个模拟器已启动，用设备 UDID 替换下列命令的 `booted` 和 `ios`。

```sh
impasto run -d ios --debug
xcrun simctl openurl booted impasto-demo://record/42
```

验证冷启动链接时，先执行 `xcrun simctl terminate booted dev.impasto.fieldnotes` 再打开链接。运行中打开 `impasto-demo://record/43` 应切换到记录 43。需要验收 Universal Link 时，在 `[ios].associated_domains` 中设置应用有权限使用的域名，并在该域发布与应用签名相符的 AASA。使用 `https://<关联域名>/records/42` 验证冷启动和运行中路由。

1. 在详情页输入中文、日文或带组合符号的备注，验证候选词替换、选区和退格。用 VoiceOver 定位备注并激活按钮。
2. 从文件选择器中选一个文件，再打开系统分享面板。取消选择或分享后，应用返回当前记录。
3. 返回主屏后结束应用进程，再从桌面图标打开。确认记录、滚动位置和未手动保存的短备注恢复。
4. 在完成开发签名的 iPhone 或 iPad 上检查相机授权。使用新安装的应用分别验证拒绝、授权和从设置修改权限后的结果。
