//! Background work on a copy of a document.
//!
//! An `Editor` is not `Sync`, so a document is only ever used by one thread at a time. Work that
//! should not block the UI (autosave, exports, scripts) therefore runs on a *snapshot*: a separate
//! document holding a copy of the sprite (cel buffers are shared copy-on-write, so this is cheap)
//! and the selection. The UI thread keeps using the real document; a job that changes its
//! snapshot, like a script, hands the result back with `pixmith_job_apply`.

use crate::document::Editor;

/// Copies the sprite and selection of `doc` into a new, independent document (no undo history).
/// Release it with `pixmith_free`. Null for a null `doc`.
#[no_mangle]
pub unsafe extern "C" fn pixmith_snapshot(doc: *mut Editor) -> *mut Editor {
    crate::ffi::guarded("pixmith_snapshot", doc, || {
        let Some(e) = crate::ffi::editor(doc) else { return std::ptr::null_mut() };
        let mut copy = Editor::new(e.sprite.clone());
        copy.sel.mask = e.sel.mask.clone();
        Box::into_raw(Box::new(copy))
    })
}

/// Takes the result of work done on a snapshot as one undo step of `doc`, named like the snapshot's
/// own history step (e.g. a script's label). Returns false, recording nothing, when the snapshot
/// still equals the document.
#[no_mangle]
pub unsafe extern "C" fn pixmith_job_apply(doc: *mut Editor, job: *mut Editor) -> bool {
    crate::ffi::guarded("pixmith_job_apply", doc, || {
        let (Some(e), Some(j)) = (crate::ffi::editor(doc), job.as_mut()) else { return false };
        j.sprite.flush_links();
        if j.sprite == e.sprite && j.sel.mask == e.sel.mask {
            return false;
        }
        let label = j.history_label(0).map(str::to_string);
        let (after, mask) = (j.sprite.clone(), j.sel.mask.clone());
        e.checkpoint();
        e.sprite = after;
        e.sel.mask = mask;
        if let Some(label) = label {
            e.label_last(&label);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sprite;
    use std::ffi::CString;

    const RED: [u8; 4] = [255, 0, 0, 255];

    #[test]
    fn snapshots_are_independent_documents() {
        let mut e = Editor::new(Sprite::new(4, 4));
        e.sprite.draw_line(0, 0, 0, 0, 1, 1, RED, 1);
        unsafe {
            let snap = pixmith_snapshot(&mut e);
            assert!(!snap.is_null());
            assert_eq!((*snap).sprite.render(0), e.sprite.render(0));
            assert_eq!((*snap).history_len(), (0, 0));
            (*snap).sprite.draw_line(0, 0, 3, 3, 3, 3, RED, 1);
            assert_ne!((*snap).sprite.render(0), e.sprite.render(0));
            crate::ffi::document::pixmith_free(snap);
            assert!(pixmith_snapshot(std::ptr::null_mut()).is_null());
        }
    }

    #[test]
    fn a_script_result_is_applied_as_one_labeled_step() {
        let mut e = Editor::new(Sprite::new(4, 4));
        let src = CString::new("sprite:setPixel(1, 1, 0xFF0000FF) app.undoLabel('Red dot')").unwrap();
        unsafe {
            let job = pixmith_snapshot(&mut e);
            let mut out = std::ptr::null_mut();
            assert!(crate::scripting::pixmith_run_script_at(job, src.as_ptr(), 0, 0, &mut out));
            crate::ffi::pixmith_string_free(out);
            assert_eq!(e.sprite.render(0), vec![0; 64], "the document is untouched while the job runs");
            assert!(pixmith_job_apply(&mut e, job));
            assert_eq!(&e.sprite.render(0)[(4 + 1) * 4..][..4], &RED);
            assert_eq!(e.history_len(), (1, 0));
            assert_eq!(e.history_label(0), Some("Red dot"));
            assert!(!pixmith_job_apply(&mut e, job), "applying twice changes nothing");
            assert_eq!(e.history_len(), (1, 0));
            crate::ffi::document::pixmith_free(job);
            assert!(e.undo());
            assert_eq!(e.sprite.render(0), vec![0; 64]);
        }
    }

    #[test]
    fn unchanged_jobs_record_nothing() {
        let mut e = Editor::new(Sprite::new(2, 2));
        unsafe {
            let job = pixmith_snapshot(&mut e);
            assert!(!pixmith_job_apply(&mut e, job));
            assert!(!pixmith_job_apply(&mut e, std::ptr::null_mut()));
            crate::ffi::document::pixmith_free(job);
        }
        assert_eq!(e.history_len(), (0, 0));
    }

    #[test]
    fn snapshots_can_be_saved_from_another_thread() {
        let mut e = Editor::new(Sprite::new(8, 8));
        e.sprite.draw_line(0, 0, 0, 0, 7, 7, RED, 1);
        let dir = std::env::temp_dir().join(format!("pixmith-jobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.aseprite");
        let snap = unsafe { pixmith_snapshot(&mut e) } as usize;
        let p = path.clone();
        let saved = std::thread::spawn(move || unsafe {
            let c = CString::new(p.to_str().unwrap()).unwrap();
            let ok = crate::ffi::document::pixmith_save(snap as *const Editor, c.as_ptr());
            crate::ffi::document::pixmith_free(snap as *mut Editor);
            ok
        })
        .join()
        .unwrap();
        assert!(saved);
        // The original keeps changing independently of the saved copy.
        let before = e.sprite.render(0);
        e.sprite.draw_line(0, 0, 7, 0, 7, 0, RED, 1);
        let back = crate::formats::ase::read(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back.render(0), before);
        assert_ne!(back.render(0), e.sprite.render(0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
