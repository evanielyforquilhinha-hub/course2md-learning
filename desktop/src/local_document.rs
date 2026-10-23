//! Open generated HTML as a local document and report launch failures.
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

fn html_target(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute() && path.is_file(),
        course2md::tr!("讲义文件不存在或已移动，请查看保存位置。")
    );
    ensure!(
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html")),
        course2md::tr!("讲义文件格式无效，请查看保存位置。")
    );
    // Keep native Unicode, spaces, % and # intact. A percent-encoded file URL
    // is not a native ShellExecute document target on every Windows install.
    Ok(path.to_path_buf())
}

pub fn open(path: &Path) -> Result<()> {
    let path = html_target(path)?;
    open_native(&path)
}

#[cfg(windows)]
fn open_native(path: &Path) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            owner: *mut std::ffi::c_void,
            verb: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut std::ffi::c_void, flags: u32) -> i32;
        fn CoUninitialize();
    }
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // This runs on the dedicated IO thread; browser handlers may require COM.
    let initialized = unsafe { CoInitializeEx(ptr::null_mut(), 2) } >= 0;
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    };
    if initialized {
        unsafe { CoUninitialize() };
    }
    ensure!(
        result > 32,
        course2md::tr!(
            "系统无法打开讲义（错误码 {}），请通过文件位置打开。",
            result
        )
    );
    Ok(())
}

#[cfg(not(windows))]
fn open_native(path: &Path) -> Result<()> {
    let status = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })
    .arg(path)
    .status()?;
    ensure!(
        status.success(),
        course2md::tr!("系统无法打开讲义，请通过文件位置打开。")
    );
    Ok(())
}

impl crate::Desktop {
    pub(crate) fn open_study_document(&mut self, path: PathBuf, cx: &mut gpui::Context<Self>) {
        self.open_study_document_with_feedback(path, None, cx);
    }

    pub(crate) fn open_study_document_with_feedback(
        &mut self,
        path: PathBuf,
        dialog_revision: Option<u64>,
        cx: &mut gpui::Context<Self>,
    ) {
        let pending = course2md::tr!("正在打开学习讲义…").to_owned();
        self.message = Some(pending.clone());
        if let Some(revision) = dialog_revision {
            self.set_study_open_status(revision, &path, pending, false);
        }
        let target = path.clone();
        let worker = crate::spawn_blocking_io(move || open(&target));
        cx.spawn(async move |this, cx| {
            let result = worker.recv().await;
            let _ = this.update(cx, |this, cx| {
                let (message, failed) = match result {
                    Ok(Ok(())) => (
                        course2md::tr!("已请求浏览器打开讲义，可使用「查看讲义文件」定位文件。")
                            .into(),
                        false,
                    ),
                    Ok(Err(error)) => (error.to_string(), true),
                    Err(_) => (crate::interrupted().into(), true),
                };
                if let Some(revision) = dialog_revision {
                    this.set_study_open_status(revision, &path, message.clone(), failed);
                }
                this.message = Some(message);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_html_target_preserves_unicode_spaces_and_reserved_url_characters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("学习 #1 100%.html");
        std::fs::write(&path, "<p>fixture</p>").unwrap();
        assert_eq!(html_target(&path).unwrap(), path);
        assert!(html_target(&dir.path().join("missing.html")).is_err());
        let script = dir.path().join("not-a-document.exe");
        std::fs::write(&script, "fixture").unwrap();
        assert!(html_target(&script).is_err());
    }
}
