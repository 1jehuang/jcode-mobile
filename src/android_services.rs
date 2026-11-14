//! JNI bridge to Android's platform surfaces. Java owns Activity lifecycle and permissions.
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JByteArray, JObject, JString, JValue},
};
use std::sync::{Arc, Mutex};

struct Bridge {
    vm: JavaVM,
    helper: GlobalRef,
}
static BRIDGE: Mutex<Option<Arc<Bridge>>> = Mutex::new(None);

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_AndroidServices_nativeInit(
    env: JNIEnv,
    helper: JObject,
) {
    match (env.get_java_vm(), env.new_global_ref(helper)) {
        (Ok(vm), Ok(helper)) => *BRIDGE.lock().unwrap() = Some(Arc::new(Bridge { vm, helper })),
        _ => log::error!("Android service initialization failed"),
    }
}

fn with<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    // Never hold the bridge lock while dispatching to Java's UI thread.
    let b = BRIDGE
        .lock()
        .map_err(|_| "Android service lock poisoned")?
        .clone()
        .ok_or("Android services are not initialized")?;
    let mut env = b.vm.attach_current_thread().map_err(|e| e.to_string())?;
    let result = env.with_local_frame(16, |env| f(env, b.helper.as_obj()));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err("Android platform operation failed".into());
    }
    result.map_err(|e| e.to_string())
}

pub fn call_bool(method: &str, text: &str) -> bool {
    with(|env, obj| {
        let text = env.new_string(text)?;
        env.call_method(
            obj,
            method,
            "(Ljava/lang/String;)Z",
            &[JValue::Object(text.as_ref())],
        )?
        .z()
    })
    .unwrap_or_else(|e| {
        log::warn!("Android {method}: {e}");
        false
    })
}
pub fn haptic(kind: i32) {
    let _ = with(|env, obj| {
        env.call_method(obj, "haptic", "(I)V", &[JValue::Int(kind)])
            .map(|_| ())
    });
}
pub fn pick(camera: bool, limit: usize) -> bool {
    with(|env, obj| {
        env.call_method(
            obj,
            "pick",
            "(ZI)Z",
            &[
                JValue::Bool(camera as u8),
                JValue::Int(limit.clamp(1, 20) as i32),
            ],
        )?
        .z()
    })
    .unwrap_or(false)
}
pub fn settings() -> (f32, bool, bool) {
    with(|env, obj| {
        let scale = env.call_method(obj, "fontScale", "()F", &[])?.f()?;
        let reduce = env.call_method(obj, "reduceMotion", "()Z", &[])?.z()?;
        let light = env.call_method(obj, "isLight", "()Z", &[])?.z()?;
        Ok((scale.clamp(0.5, 3.0), reduce, light))
    })
    .unwrap_or((1.0, false, false))
}
pub fn audio_start() -> Result<(), String> {
    let status = with(|env, obj| env.call_method(obj, "audioStart", "()I", &[])?.i())?;
    match status {
        0 => Ok(()),
        1 => Err("Microphone permission requested. Allow access, then tap record again.".into()),
        -2 => Err(
            "Microphone permission denied. Enable microphone access in Android app settings."
                .into(),
        ),
        _ => Err("Microphone unavailable or permission denied".into()),
    }
}
pub fn audio_read() -> Result<Vec<u8>, String> {
    with(|env, obj| {
        let array = env.call_method(obj, "audioRead", "()[B", &[])?.l()?;
        env.convert_byte_array(JByteArray::from(array))
    })
}
pub fn audio_stop() {
    let _ = with(|env, obj| env.call_method(obj, "audioStop", "()V", &[]).map(|_| ()));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_AndroidServices_nativeMedia(
    env: JNIEnv,
    _: JObject,
    status: i32,
    bytes: JByteArray,
) {
    use crate::media::{Event, emit};
    match status {
        0 => match env.get_array_length(&bytes) {
            Ok(n) if n > 0 && n as usize <= jcode_mobile_core::session::MAX_IMAGE_BYTES => {
                if let Ok(data) = env.convert_byte_array(&bytes) {
                    emit(Event::Image(jcode_mobile_core::SessionImage::new(
                        "image/jpeg",
                        data,
                    )));
                }
            }
            _ => emit(Event::Unavailable),
        },
        1 => emit(Event::Done),
        2 => emit(Event::Cancelled),
        _ => emit(Event::Unavailable),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_AndroidServices_nativeResume(
    mut env: JNIEnv,
    _: JObject,
    url: JString,
) {
    crate::system::NEEDS_REFRESH.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::notify::android_foreground();
    if !url.is_null() {
        if let Ok(url) = env.get_string(&url) {
            crate::ios_ext::handle_urls(vec![url.into()]);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_AndroidServices_nativeReply(
    mut env: JNIEnv,
    _: JObject,
    session: JString,
    text: JString,
) {
    if let (Ok(session), Ok(text)) = (env.get_string(&session), env.get_string(&text)) {
        crate::notify::android_reply(session.into(), text.into());
    }
}
