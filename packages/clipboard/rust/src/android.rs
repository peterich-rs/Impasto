use impasto::os::android::AndroidContext;
use jni::objects::{JString, JValue};
use jni::{jni_sig, jni_str};

pub(super) fn get(platform: &AndroidContext) -> String {
    platform
        .with_view_now(|env, view| {
            let text = env
                .call_method(
                    view,
                    jni_str!("readClipboard"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            env.cast_local::<JString>(text)?.try_to_string(env)
        })
        .unwrap_or_default()
}

pub(super) fn set(platform: &AndroidContext, text: &str) {
    let _ = platform.with_view_now(|env, view| {
        let context = env.call_method(view, jni_str!("getContext"), jni_sig!("()Landroid/content/Context;"), &[])?.l()?;
        let service = env.new_string("clipboard")?;
        let clipboard = env.call_method(
            &context, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::Object(&service)],
        )?.l()?;
        if !clipboard.is_null() {
            let label = env.new_string("text")?;
            let text = env.new_string(text)?;
            let clip = env.call_static_method(
                jni_str!("android/content/ClipData"), jni_str!("newPlainText"),
                jni_sig!("(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"),
                &[JValue::Object(&label), JValue::Object(&text)],
            )?.l()?;
            env.call_method(
                &clipboard, jni_str!("setPrimaryClip"), jni_sig!("(Landroid/content/ClipData;)V"), &[JValue::Object(&clip)]
            )?;
        }
        Ok(())
    });
}
