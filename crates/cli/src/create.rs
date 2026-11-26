use crate::{
    app::{App, validate_name},
    build::copy_tree,
    ios, sync, toolchain,
};
use anyhow::{Context, Result, bail};
use std::{fmt::Write as _, fs, path::Path};

const MAIN: &str = "import { boot } from \"@impasto/solid-renderer\";\nimport { View } from \"@impasto/solid-ui\";\n\nexport default function Main() {\n  return <View />;\n}\n\nboot(Main);\n";
const TSCONFIG: &str = "{\n  \"extends\": \"./.impasto/tsconfig.generated.json\"\n}\n";

pub fn create(name: &str, with_rust: bool) -> Result<()> {
    validate_name(name)?;
    let root = Path::new(name);
    if root.exists() {
        bail!("{} 已存在", root.display());
    }
    fs::create_dir(root)?;
    fs::create_dir(root.join("src"))?;
    fs::create_dir(root.join("assets"))?;
    fs::write(
        root.join("app.toml"),
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ntargets = [\"macos\", \"android\", \"ios\"]\n"
        ),
    )?;
    fs::write(root.join("src/main.tsx"), MAIN)?;
    let app = App::load(root)?;
    if with_rust {
        rust_crate(&app)?;
    }
    let app = App::load(root)?;
    scaffold(&app)?;
    sync::sync(&app, None)
}

pub fn init_rust() -> Result<()> {
    let app = App::current()?;
    rust_crate(&app)?;
    sync::sync(&App::load(&app.root)?, None)
}

pub(crate) fn rust_crate(app: &App) -> Result<()> {
    let rust = app.root.join("rust");
    if rust.exists() {
        bail!("{} 已存在", rust.display());
    }
    fs::create_dir_all(rust.join("src"))?;
    fs::write(
        rust.join("Cargo.toml"),
        format!(
            "[package]\nname = {:?}\nversion = {:?}\nedition = \"2024\"\npublish = false\n\n[dependencies]\nimpasto = {{ version = {:?}, registry = \"impasto\" }}\n",
            format!("{}-native", app.name),
            app.version,
            impasto::VERSION
        ),
    )?;
    fs::write(
        rust.join("src/lib.rs"),
        format!(
            "#[impasto::module(name = \"App\", specifier = \"@app/{}/native\")]\nstruct App;\n\n#[impasto::export]\nimpl App {{\n    pub fn greet() -> String {{\n        \"Hello from Rust\".into()\n    }}\n}}\n\npub fn register(registrar: &mut impasto::PluginRegistrar) {{\n    registrar.add(App::plugin());\n}}\n",
            app.name,
        ),
    )?;
    Ok(())
}

pub(crate) fn scaffold(app: &App) -> Result<()> {
    let framework = toolchain::framework()?;
    write_new(
        &app.root.join(".gitignore"),
        "node_modules/\n.impasto/\n.cargo/config.toml\nrust/Cargo.lock\nbuild/\n.gradle/\n.kotlin/\nlocal.properties\nxcuserdata/\n",
    )?;
    write_new(&app.root.join("tsconfig.json"), TSCONFIG)?;
    android(app, &framework)?;
    for platform in ["ios", "macos"] {
        darwin(app, platform, &framework)?;
    }
    Ok(())
}

fn android(app: &App, framework: &Path) -> Result<()> {
    let root = app.root.join("android");
    let template = framework.join("crates/embedder/android/apk");
    if root.is_dir() {
        return Ok(());
    }
    fs::create_dir(&root)?;
    for name in [
        "gradlew",
        "gradlew.bat",
        "gradle.properties",
        "build.gradle.kts",
        "app/build.gradle.kts",
    ] {
        let target = root.join(name);
        fs::create_dir_all(target.parent().context("缺少 Android 目录")?)?;
        fs::copy(template.join(name), target)?;
    }
    let build = root.join("build.gradle.kts");
    let script = fs::read_to_string(&build)?;
    fs::write(
        build,
        format!(
            "{script}\nval output = rootProject.file(\"../build/android/gradle\")\nsubprojects {{ layout.buildDirectory.set(output.resolve(name)) }}\n"
        ),
    )?;
    copy_tree(&template.join("gradle"), &root.join("gradle"), &[])?;
    write_new(
        &root.join("settings.gradle.kts"),
        include_str!("../templates/android/settings.gradle.kts"),
    )?;
    write_new(
        &root.join("app/src/main/kotlin/dev/impasto/runner/MainActivity.kt"),
        include_str!("../templates/android/MainActivity.kt"),
    )?;
    let mut permissions = String::new();
    for permission in &app.android.permissions {
        writeln!(
            permissions,
            "    <uses-permission android:name=\"{permission}\" />"
        )?;
    }
    let mut filters = String::new();
    for link in &app.android.deep_links {
        let path = link
            .path_prefix
            .as_deref()
            .map_or_else(String::new, |value| {
                format!(
                    " android:pathPrefix=\"{}\"",
                    crate::build::escape_xml(value)
                )
            });
        writeln!(
            filters,
            r#"            <intent-filter>
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <category android:name="android.intent.category.BROWSABLE" />
                <data android:scheme="{}" android:host="{}"{path} />
            </intent-filter>"#,
            link.scheme, link.host
        )?;
    }
    let manifest = fs::read_to_string(template.join("app/src/main/AndroidManifest.xml"))?
        .replace(
            "    <uses-permission android:name=\"android.permission.INTERNET\" />\n",
            &permissions,
        )
        .replace("impasto counter", "${impastoAppLabel}")
        .replace(
            "dev.impasto.host.ImpastoActivity",
            "dev.impasto.runner.MainActivity",
        )
        .replace(
            "        </activity>",
            &format!("{filters}        </activity>"),
        );
    write_new(&root.join("app/src/main/res/.gitkeep"), "")?;
    write_new(&root.join("app/src/main/AndroidManifest.xml"), &manifest)
}

fn darwin(app: &App, platform: &str, framework: &Path) -> Result<()> {
    let project = app.root.join(platform);
    let runner = project.join("Runner");
    if project.is_dir() {
        return Ok(());
    }
    let ios = platform == "ios";
    let product = if ios { "Impasto" } else { app.name.as_str() };
    let mut extra_groups = String::from(", A00000000000000000000020");
    let mut extra_resources = String::new();
    let mut extra_references = String::from(
        "A00000000000000000000020 = {isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Runner/Info.plist; sourceTree = \"<group>\"; };",
    );
    fs::create_dir_all(&runner)?;
    if ios {
        extra_groups.push_str(", A00000000000000000000021, A00000000000000000000022");
        extra_resources.push_str("A00000000000000000000023, A00000000000000000000024");
        extra_references.push_str(r#"
        A00000000000000000000021 = {isa = PBXFileReference; lastKnownFileType = file.storyboard; path = Runner/LaunchScreen.storyboard; sourceTree = "<group>"; };
        A00000000000000000000022 = {isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Runner/PrivacyInfo.xcprivacy; sourceTree = "<group>"; };
        A00000000000000000000023 = {isa = PBXBuildFile; fileRef = A00000000000000000000021; };
        A00000000000000000000024 = {isa = PBXBuildFile; fileRef = A00000000000000000000022; };"#);
        let host = framework.join("crates/embedder/ios");
        let launch = app.ios.launch_storyboard.as_ref().map_or_else(
            || host.join("host/LaunchScreen.storyboard"),
            |path| app.root.join(path),
        );
        fs::copy(launch, runner.join("LaunchScreen.storyboard"))?;
        ios::write_privacy_manifest(
            &host.join("template/PrivacyInfo.xcprivacy"),
            app.ios
                .privacy_manifest
                .as_ref()
                .map(|path| app.root.join(path))
                .as_deref(),
            &runner.join("PrivacyInfo.xcprivacy"),
        )?;
        write_new(&runner.join("Info.plist"), &ios::app_plist(app, &host)?)?;
        write_new(
            &runner.join("App.swift"),
            include_str!("../templates/darwin/ios.swift"),
        )?;
    } else {
        extra_groups.push_str(", A00000000000000000000021");
        extra_references.push_str(r#"
        A00000000000000000000021 = {isa = PBXFileReference; lastKnownFileType = text.plist.entitlements; path = Runner/Runner.entitlements; sourceTree = "<group>"; };"#);
        let mut entries = String::from(
            r#"
<key>CFBundleExecutable</key><string>$(EXECUTABLE_NAME)</string>
<key>CFBundleIdentifier</key><string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
<key>CFBundleName</key><string>$(PRODUCT_NAME)</string>
<key>CFBundleShortVersionString</key><string>$(MARKETING_VERSION)</string>
<key>CFBundleVersion</key><string>$(CURRENT_PROJECT_VERSION)</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>$(MACOSX_DEPLOYMENT_TARGET)</string>
<key>NSHighResolutionCapable</key><true/>
"#,
        );
        for (key, value) in &app.macos.usage_descriptions {
            writeln!(
                entries,
                "<key>{key}</key><string>{}</string>",
                crate::build::escape_xml(value)
            )?;
        }
        write_new(&runner.join("Info.plist"), &ios::plist_xml(&entries))?;
        let entitlements = app
            .macos
            .entitlements
            .iter()
            .map(|(key, value)| format!("<key>{key}</key><{value}/>\n"))
            .collect::<String>();
        write_new(
            &runner.join("Runner.entitlements"),
            &ios::plist_xml(&entitlements),
        )?;
        write_new(
            &runner.join("App.swift"),
            include_str!("../templates/darwin/macos.swift"),
        )?;
    }
    let pbx = include_str!("../templates/darwin/project.pbxproj")
        .replace("@PRODUCT@", product)
        .replace("@PLATFORM@", platform)
        .replace("@SDK@", if ios { "iphoneos" } else { "macosx" })
        .replace("@EXTRA_GROUPS@", &extra_groups)
        .replace("@EXTRA_REFERENCES@", &extra_references)
        .replace("@EXTRA_RESOURCES@", &extra_resources);
    let xcode = project.join("Runner.xcodeproj");
    write_new(&xcode.join("project.pbxproj"), &pbx)?;
    write_new(
        &xcode.join("xcshareddata/xcschemes/Runner.xcscheme"),
        &include_str!("../templates/darwin/Runner.xcscheme").replace("@PRODUCT@", product),
    )?;
    let platform_settings = if ios {
        "SUPPORTED_PLATFORMS = iphoneos iphonesimulator\nIPHONEOS_DEPLOYMENT_TARGET = 15.0\n"
    } else {
        "MACOSX_DEPLOYMENT_TARGET = 13.0\nEXECUTABLE_NAME = impasto\nCODE_SIGN_ENTITLEMENTS = Runner/Runner.entitlements\n"
    };
    let settings = format!(
        "PRODUCT_NAME = {product}\nINFOPLIST_FILE = Runner/Info.plist\nSWIFT_VERSION = 5.0\nALWAYS_SEARCH_USER_PATHS = NO\nENABLE_USER_SCRIPT_SANDBOXING = NO\n{platform_settings}#include \"../../.impasto/darwin/{platform}/Generated.xcconfig\"\n"
    );
    for config in ["Debug", "Release"] {
        write_new(
            &project.join(format!("Config/{config}.xcconfig")),
            &format!(
                "{settings}#include? \"../../.impasto/darwin/{platform}/{config}.xcconfig\"\n"
            ),
        )?;
    }
    Ok(())
}

fn write_new(path: &Path, content: &str) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path.parent().context("缺少模板目标目录")?)?;
        fs::write(path, content)?;
    }
    Ok(())
}
