//! Activity bootstrap glue for Android's composing InputConnection.
//! JNI callbacks enqueue edits. Only GPUI's native UI loop mutates text state.
use gpui_platform::android::{AndroidPlatformServices, AndroidTextInput, dispatch_text_input};
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};
use std::sync::{Arc, Mutex};

struct Activity {
    vm: JavaVM,
    object: GlobalRef,
}
static ACTIVITY: Mutex<Option<Arc<Activity>>> = Mutex::new(None);

fn with_activity<T>(
    operation: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>,
) -> Option<T> {
    let activity = ACTIVITY.lock().ok()?.clone()?;
    let mut env = activity.vm.attach_current_thread().ok()?;
    let result = env.with_local_frame(16, |env| operation(env, activity.object.as_obj()));
    if env.exception_check().unwrap_or(true) {
        // Clipboard/editor exceptions can contain private text. Never describe them.
        let _ = env.exception_clear();
        return None;
    }
    result.ok()
}

struct Services;
impl AndroidPlatformServices for Services {
    fn open_url(&self, url: &str) {
        let _ = with_activity(|env, activity| {
            let url = env.new_string(url)?;
            env.call_method(
                activity,
                "gpuiOpenUrl",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(url.as_ref())],
            )?
            .z()
        });
    }
    fn read_clipboard(&self) -> Option<String> {
        with_activity(|env, activity| {
            let text = env
                .call_method(activity, "gpuiReadClipboard", "()Ljava/lang/String;", &[])?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let text = JString::from(text);
            Ok(Some(env.get_string(&text)?.into()))
        })
        .flatten()
    }
    fn write_clipboard(&self, text: &str) {
        let _ = with_activity(|env, activity| {
            let text = env.new_string(text)?;
            env.call_method(
                activity,
                "gpuiWriteClipboard",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(text.as_ref())],
            )?
            .z()
        });
    }
    fn set_keyboard_visible(&self, visible: bool) -> bool {
        with_activity(|env, activity| {
            env.call_method(
                activity,
                "gpuiKeyboardVisible",
                "(Z)Z",
                &[JValue::Bool(u8::from(visible))],
            )?
            .z()
        })
        .unwrap_or(false)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_JcodeActivity_nativeInitInput(
    env: JNIEnv,
    activity: JObject,
) {
    if let (Ok(vm), Ok(object), Ok(mut state)) = (
        env.get_java_vm(),
        env.new_global_ref(activity),
        ACTIVITY.lock(),
    ) {
        *state = Some(Arc::new(Activity { vm, object }));
        gpui_platform::android::set_platform_services(Arc::new(Services));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_JcodeActivity_nativeWindowInsets(
    _: JNIEnv,
    _: JClass,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    ime_left: i32,
    ime_top: i32,
    ime_right: i32,
    ime_bottom: i32,
) {
    gpui_platform::android::dispatch_window_insets(
        [left, top, right, bottom],
        [ime_left, ime_top, ime_right, ime_bottom],
    );
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_jcode_mobile_GpuiInput_nativeEdit(
    mut env: JNIEnv,
    _: JClass,
    kind: i32,
    text: JString,
    before: i32,
    after: i32,
) {
    let edit = match kind {
        0 | 1 => {
            if text.is_null() {
                return;
            }
            let Ok(text) = env.get_string(&text) else {
                return;
            };
            let text: String = text.into();
            if kind == 0 {
                AndroidTextInput::Commit(text)
            } else {
                AndroidTextInput::Compose(text)
            }
        }
        2 => AndroidTextInput::FinishComposition,
        3 if before >= 0 && after >= 0 => AndroidTextInput::DeleteSurrounding {
            before: before as usize,
            after: after as usize,
        },
        4 => AndroidTextInput::Enter,
        _ => return,
    };
    dispatch_text_input(edit);
}
