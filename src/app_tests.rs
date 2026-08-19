use super::*;
use crate::project::{DocEntry, DocRole, ProjectManifest};
use ropey::Rope;

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pstar-app-{tag}-{}-{id}", std::process::id()))
}

#[test]
fn save_failure_preserves_every_copy_then_alternate_save_succeeds() {
    let dir = scratch_dir("save-failure");
    std::fs::create_dir_all(&dir).unwrap();
    let failed_path = dir.join("chapter.md");
    let alternate_path = dir.join("recovered.md");
    let disk_text = "previous good manuscript";
    std::fs::write(&failed_path, disk_text).unwrap();

    let mut app = App::new(Some(failed_path.clone())).unwrap();
    let end = app.buf.len_chars();
    app.buf.insert(end, " with unsaved changes");
    let expected_text = app.buf.rope.to_string();
    let recovery_root = dir.join("recovery");
    app.backup_depth = 2;
    app.backup_root = Some(recovery_root.clone());
    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&failed_path), "unused");
    app.maybe_autosave();
    let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
    assert_eq!(
        std::fs::read_to_string(&recovery_path).unwrap(),
        expected_text
    );

    // A directory at the atomic helper's sibling temp path deterministically
    // simulates a write failure while a prior good manuscript exists.
    let mut temporary = failed_path.clone().into_os_string();
    temporary.push(".tmp~");
    std::fs::create_dir(PathBuf::from(temporary)).unwrap();

    // Exercise the actual command arm, not just Buffer::save directly.
    app.execute(Cmd::Save);

    assert_eq!(app.buf.rope.to_string(), expected_text);
    assert_eq!(app.buf.path.as_deref(), Some(failed_path.as_path()));
    assert!(app.buf.dirty);
    assert_eq!(std::fs::read_to_string(&failed_path).unwrap(), disk_text);
    assert_eq!(
        std::fs::read_to_string(failed_path.with_extension("md.bak")).unwrap(),
        disk_text
    );
    assert_eq!(
        std::fs::read_to_string(&recovery_path).unwrap(),
        expected_text
    );
    assert!(
        app.status_msg
            .as_deref()
            .unwrap()
            .starts_with("Save failed:")
    );
    match &app.mode {
        Mode::Input {
            label,
            action: InputAction::SaveAs,
            ..
        } => {
            assert!(label.contains("Save failed:"));
            assert!(label.contains("Alternate save path"));
        }
        _ => panic!("save failure did not open the alternate-path prompt"),
    }

    if let Mode::Input { value, .. } = &mut app.mode {
        *value = alternate_path.to_string_lossy().into_owned();
    }
    app.handle_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.path.as_deref(), Some(alternate_path.as_path()));
    assert!(!app.buf.dirty);
    assert!(
        !recovery_path.exists(),
        "successful Save As left the old recovery journal"
    );
    assert_eq!(std::fs::read_to_string(&failed_path).unwrap(), disk_text);
    assert_eq!(
        std::fs::read_to_string(&alternate_path).unwrap(),
        expected_text
    );
    assert_eq!(
        app.status_msg.as_deref(),
        Some(format!("Saved {}", app.buf.file_name()).as_str())
    );
    let save_as_backups = recovery_root
        .join("backups")
        .join(crate::paths::path_key(&alternate_path));
    let backup = std::fs::read_dir(save_as_backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), expected_text);

    // finish_save persists the pane session under the normal metadata root;
    // remove this test's path-keyed artifact as well as its manuscript.
    if let Some(sessions) = crate::paths::sessions() {
        let session = sessions.join(format!("{}.json", crate::paths::path_key(&alternate_path)));
        let _ = std::fs::remove_file(session);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn successful_save_paths_rotate_to_exact_backup_depth() {
    let dir = scratch_dir("rolling-save-seams");
    let source = dir.join("chapter.md");
    let recovery_root = dir.join("recovery");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&source, "saved").unwrap();

    let mut app = App::new(Some(source.clone())).unwrap();
    app.backup_depth = 2;
    app.backup_root = Some(recovery_root.clone());

    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    let first_end = app.buf.len_chars();
    app.buf.insert(first_end, " manually");
    app.save();

    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    let second_end = app.buf.len_chars();
    app.buf.insert(second_end, " twice");
    app.save();

    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    app.autosave = Duration::from_nanos(1);
    let autosave_end = app.buf.len_chars();
    app.buf.insert(autosave_end, " and automatically");
    app.maybe_autosave();

    let backup_dir = recovery_root
        .join("backups")
        .join(crate::paths::path_key(&source));
    let mut backups = std::fs::read_dir(backup_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    backups.sort();
    assert_eq!(backups.len(), app.backup_depth);
    assert_eq!(
        std::fs::read_to_string(&backups[0]).unwrap(),
        "saved manually twice"
    );
    assert_eq!(
        std::fs::read_to_string(&backups[1]).unwrap(),
        "saved manually twice and automatically"
    );
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "saved manually twice and automatically"
    );
    assert_eq!(app.status_msg.as_deref(), Some("Autosaved"));
    assert!(!app.buf.dirty);

    if let Some(sessions) = crate::paths::sessions() {
        let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
        let _ = std::fs::remove_file(session);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rolling_backup_failure_warns_without_turning_save_into_failure() {
    let dir = scratch_dir("rolling-warning");
    let source = dir.join("chapter.md");
    let blocked_root = dir.join("not-a-directory");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&source, "old").unwrap();
    std::fs::write(&blocked_root, "blocks directory creation").unwrap();

    let mut app = App::new(Some(source.clone())).unwrap();
    app.backup_depth = 1;
    app.backup_root = Some(blocked_root);
    let old_len = app.buf.len_chars();
    app.buf.delete(0..old_len);
    app.buf.insert(0, "new saved text");
    app.save();

    assert!(!app.buf.dirty);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "new saved text");
    let message = app.status_msg.as_deref().unwrap();
    assert!(message.starts_with("Saved chapter.md;"));
    assert!(message.contains("rolling backup failed:"));

    if let Some(sessions) = crate::paths::sessions() {
        let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
        let _ = std::fs::remove_file(session);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn idle_tick_journals_unnamed_buffer_when_autosave_is_disabled() {
    let dir = scratch_dir("recovery-unnamed");
    let mut app = App::new(None).unwrap();
    app.autosave = Duration::ZERO;
    app.recovery_journals[0] = Journal::in_root(&dir, None, "untitled-idle");
    app.buf.insert(0, "new unsaved manuscript");

    app.maybe_autosave();

    let journal_path = app.recovery_journals[0].path().unwrap();
    assert!(app.buf.dirty);
    assert_eq!(
        std::fs::read_to_string(journal_path).unwrap(),
        "new unsaved manuscript"
    );
    assert!(
        journal_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("untitled-idle-")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn clean_exit_clears_journal_but_dirty_abandon_retains_it() {
    let clean_dir = scratch_dir("recovery-clean-exit");
    let mut clean_app = App::new(None).unwrap();
    clean_app.autosave = Duration::ZERO;
    clean_app.recovery_journals[0] = Journal::in_root(&clean_dir, None, "untitled-clean");
    clean_app.buf.insert(0, "saved work");
    clean_app.maybe_autosave();
    let clean_path = clean_app.recovery_journals[0].path().unwrap().to_path_buf();
    clean_app.buf.dirty = false;

    clean_app.close_or_quit();

    assert!(clean_app.quit);
    assert!(!clean_path.exists());

    let dirty_dir = scratch_dir("recovery-dirty-exit");
    let mut dirty_app = App::new(None).unwrap();
    dirty_app.autosave = Duration::ZERO;
    dirty_app.recovery_journals[0] = Journal::in_root(&dirty_dir, None, "untitled-dirty");
    dirty_app.buf.insert(0, "abandoned but recoverable");
    dirty_app.maybe_autosave();
    let dirty_path = dirty_app.recovery_journals[0].path().unwrap().to_path_buf();

    // This is called only after the user confirms abandoning changes.
    dirty_app.close_or_quit();

    assert!(dirty_app.quit);
    assert!(dirty_path.exists());
    assert_eq!(
        std::fs::read_to_string(&dirty_path).unwrap(),
        "abandoned but recoverable"
    );

    let _ = std::fs::remove_dir_all(clean_dir);
    let _ = std::fs::remove_dir_all(dirty_dir);
}

#[test]
fn startup_recovery_restores_as_one_undoable_dirty_edit() {
    let dir = scratch_dir("recovery-restore");
    let source = dir.join("chapter.md");
    let recovery_root = dir.join("recovery");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&source, "saved manuscript").unwrap();
    std::thread::sleep(Duration::from_millis(20));

    let mut app = App::new(Some(source.clone())).unwrap();
    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    app.recovery_journals[0]
        .write_if_changed(&Rope::from_str("recovered manuscript"), Instant::now())
        .unwrap();
    let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
    app.offer_recovery_for_active();

    assert!(matches!(app.mode, Mode::ConfirmRecover));
    assert!(
        !app.splash,
        "the splash must not consume the recovery answer"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::ConfirmRecover));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.rope.to_string(), "recovered manuscript");
    assert!(app.buf.dirty);
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "saved manuscript"
    );
    assert!(
        recovery_path.exists(),
        "restore must retain the journal until save"
    );

    app.execute(Cmd::Undo);
    assert_eq!(app.buf.rope.to_string(), "saved manuscript");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn simulated_crash_restart_restores_then_save_clears_journal() {
    let dir = scratch_dir("recovery-restart");
    let source = dir.join("chapter.md");
    let recovery_root = dir.join("recovery");
    let disk_text = "saved manuscript";
    let recovered_text = "saved manuscript plus unsaved work";
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&source, disk_text).unwrap();
    std::thread::sleep(Duration::from_millis(20));

    // First process: edit and reach the idle recovery tick, then disappear
    // without save/clean-exit cleanup.
    let mut crashed = App::new(Some(source.clone())).unwrap();
    crashed.autosave = Duration::ZERO;
    crashed.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    let end = crashed.buf.len_chars();
    crashed.buf.insert(end, " plus unsaved work");
    crashed.maybe_autosave();
    let recovery_path = crashed.recovery_journals[0].path().unwrap().to_path_buf();
    assert_eq!(
        std::fs::read_to_string(&recovery_path).unwrap(),
        recovered_text
    );
    assert_eq!(std::fs::read_to_string(&source).unwrap(), disk_text);
    drop(crashed);

    // Second process: reopen from disk, discover the surviving journal,
    // accept it, and keep the on-disk manuscript unchanged until save.
    let mut restarted = App::new(Some(source.clone())).unwrap();
    restarted.backup_depth = 0;
    restarted.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    restarted.offer_recovery_for_active();
    assert!(matches!(restarted.mode, Mode::ConfirmRecover));
    restarted.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

    assert_eq!(restarted.buf.rope.to_string(), recovered_text);
    assert!(restarted.buf.dirty);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), disk_text);
    assert!(recovery_path.exists());

    restarted.save();

    assert!(!restarted.buf.dirty);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), recovered_text);
    assert!(
        !recovery_path.exists(),
        "committing restored text must clear its crash journal"
    );

    if let Some(sessions) = crate::paths::sessions() {
        let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
        let _ = std::fs::remove_file(session);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn declining_startup_recovery_keeps_disk_text_and_clears_record() {
    let dir = scratch_dir("recovery-decline");
    let source = dir.join("chapter.md");
    let recovery_root = dir.join("recovery");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&source, "saved manuscript").unwrap();
    std::thread::sleep(Duration::from_millis(20));

    let mut app = App::new(Some(source.clone())).unwrap();
    app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
    app.recovery_journals[0]
        .write_if_changed(&Rope::from_str("declined manuscript"), Instant::now())
        .unwrap();
    let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
    app.offer_recovery_for_active();
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.rope.to_string(), "saved manuscript");
    assert!(!app.buf.dirty);
    assert!(!recovery_path.exists());
    assert_eq!(app.status_msg.as_deref(), Some("Recovery declined"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn app_project_initializes_as_none() {
    // R1.8: backward compat — app starts with no project when opening
    // a bare file. Task 1.2.
    let app = App::new(None).expect("App creation should succeed");
    assert!(app.project.is_none(), "Project should be None at startup");
}

#[test]
fn project_field_holds_loaded_project() {
    // Task 1.2: the project field exists and can store a Project.
    // We don't test the full load flow here (that's in project.rs tests),
    // just that the type can hold one.
    let mut app = App::new(None).expect("App creation should succeed");
    assert!(app.project.is_none());

    // Setting a project (would be done by open_project in practice).
    // Just verify the field can hold it.
    let _can_be_set: Option<Project> = app.project.take();
    app.project = None; // Put it back
    assert!(app.project.is_none());
}

/// Helper to create a test project with sample documents.
fn setup_test_project() -> (std::path::PathBuf, Project) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("pstar-app-test-1.4-{}", id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect(&format!("Failed to create test dir: {:?}", dir));

    // Create test files.
    let doc1 = dir.join("doc1.md");
    let doc2 = dir.join("doc2.md");
    let doc3 = dir.join("doc3.md");
    std::fs::write(&doc1, "First document content").expect("Failed to write doc1");
    std::fs::write(&doc2, "Second document content").expect("Failed to write doc2");
    std::fs::write(&doc3, "Third document content").expect("Failed to write doc3");

    // Create a project manifest.
    let manifest_path = dir.join("test.pstarproj");
    let project = Project {
        manifest_path: manifest_path.clone(),
        manifest: crate::project::ProjectManifest {
            name: "Test Project".to_string(),
            docs: vec![
                crate::project::DocEntry {
                    path: doc1,
                    title: "Doc 1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                crate::project::DocEntry {
                    path: doc2,
                    title: "Doc 2".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                crate::project::DocEntry {
                    path: doc3,
                    title: "Doc 3".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
            ],
            separator: crate::project::Separator::PageBreak,
        },
    };
    project.save().expect(&format!(
        "Failed to save project manifest to {:?}",
        manifest_path
    ));

    (dir, project)
}

#[test]
fn binder_move_up_at_top_is_noop() {
    // Task 1.4: moving the first document up should be a no-op.
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with first doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 0,
    };

    // Try to move up from position 0.
    app.binder_move_up();

    // Should show "Already at top" message.
    assert!(app.status_msg.as_ref().unwrap().contains("top"));

    // Order should be unchanged.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs[0].title, "Doc 1");
    assert_eq!(project.manifest.docs[1].title, "Doc 2");
}

#[test]
fn binder_move_down_at_bottom_is_noop() {
    // Task 1.4: moving the last document down should be a no-op.
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with last doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 2,
    };

    // Try to move down from last position.
    app.binder_move_down();

    // Should show "Already at bottom" message.
    assert!(app.status_msg.as_ref().unwrap().contains("bottom"));

    // Order should be unchanged.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs[2].title, "Doc 3");
}

#[test]
fn binder_move_up_reorders_and_saves() {
    // Task 1.4: moving a document up should reorder and persist atomically (R1.4).
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with second doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 1,
    };

    // Move second doc up (should become first).
    app.binder_move_up();

    // Check the new order in memory.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs[0].title, "Doc 2");
    assert_eq!(project.manifest.docs[1].title, "Doc 1");
    assert_eq!(project.manifest.docs[2].title, "Doc 3");

    // Verify it was saved (reload and check).
    let manifest_path = project.manifest_path.clone();
    let reloaded = Project::load(&manifest_path).unwrap();
    assert_eq!(reloaded.manifest.docs[0].title, "Doc 2");
    assert_eq!(reloaded.manifest.docs[1].title, "Doc 1");
}

#[test]
fn binder_move_down_reorders_and_saves() {
    // Task 1.4: moving a document down should reorder and persist atomically (R1.4).
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with first doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 0,
    };

    // Move first doc down (should become second).
    app.binder_move_down();

    // Check the new order in memory.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs[0].title, "Doc 2");
    assert_eq!(project.manifest.docs[1].title, "Doc 1");
    assert_eq!(project.manifest.docs[2].title, "Doc 3");

    // Verify it was saved.
    let manifest_path = project.manifest_path.clone();
    let reloaded = Project::load(&manifest_path).unwrap();
    assert_eq!(reloaded.manifest.docs[0].title, "Doc 2");
    assert_eq!(reloaded.manifest.docs[1].title, "Doc 1");
}

#[test]
fn project_add_doc_adds_and_saves() {
    // Task 1.4: adding a document should append to manifest and save atomically (R1.5).
    let (dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Create a new file to add.
    let new_doc = dir.join("doc4.md");
    std::fs::write(&new_doc, "New document").unwrap();

    // Add the document.
    app.project_add_doc(new_doc.to_str().unwrap());

    // Verify it was added.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs.len(), 4);
    assert_eq!(project.manifest.docs[3].title, "doc4");

    // Verify it was saved.
    let manifest_path = project.manifest_path.clone();
    let reloaded = Project::load(&manifest_path).unwrap();
    assert_eq!(reloaded.manifest.docs.len(), 4);
    assert_eq!(reloaded.manifest.docs[3].title, "doc4");
}

#[test]
fn project_add_doc_missing_file_shows_error() {
    // Task 1.4: adding a missing file should show an error (R1.5).
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Try to add a non-existent file.
    app.project_add_doc("/nonexistent/file.md");

    // Should show "File not found" error.
    assert!(app.status_msg.as_ref().unwrap().contains("not found"));

    // Project should be unchanged.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs.len(), 3);
}

#[test]
fn project_remove_doc_removes_and_saves() {
    // Task 1.4: removing a document should delete from manifest but keep file (R1.5).
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    let doc2_path = project.manifest.docs[1].path.clone();
    app.project = Some(project);

    // Enter binder mode with second doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 1,
    };

    // Remove the selected document.
    app.project_remove_doc();

    // Verify it was removed from the manifest.
    let project = app.project.as_ref().unwrap();
    assert_eq!(project.manifest.docs.len(), 2);
    assert_eq!(project.manifest.docs[0].title, "Doc 1");
    assert_eq!(project.manifest.docs[1].title, "Doc 3");

    // R1.5: verify the file still exists on disk.
    assert!(doc2_path.exists(), "File should not be deleted from disk");

    // Verify the change was saved.
    let manifest_path = project.manifest_path.clone();
    let reloaded = Project::load(&manifest_path).unwrap();
    assert_eq!(reloaded.manifest.docs.len(), 2);
}

#[test]
fn project_remove_doc_shows_clear_message() {
    // Task 1.4: removing should show clear message that file is kept (R1.5).
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with first doc selected.
    app.mode = Mode::Binder {
        entries: vec![BinderEntry {
            idx: 0,
            title: "Doc 1".to_string(),
            word_count: Some(3),
            exists: true,
            synopsis: String::new(),
        }],
        selected: 0,
    };

    // Remove the document.
    app.project_remove_doc();

    // Verify the status message mentions "file kept on disk".
    let msg = app.status_msg.as_ref().unwrap();
    assert!(
        msg.contains("kept on disk"),
        "Message should clarify file is kept: {}",
        msg
    );
}

#[test]
fn project_remove_last_doc_adjusts_selection() {
    // Task 1.4: removing the last document should adjust selection to avoid out-of-bounds.
    let (_dir, project) = setup_test_project();
    let mut app = App::new(None).unwrap();
    app.project = Some(project);

    // Enter binder mode with last doc selected.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Doc 2".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 2,
    };

    // Remove the last document.
    app.project_remove_doc();

    // Verify selection was adjusted.
    if let Mode::Binder { selected, entries } = &app.mode {
        assert_eq!(
            *selected, 1,
            "Selection should be adjusted to last valid index"
        );
        assert_eq!(entries.len(), 2);
    } else {
        panic!("Should still be in binder mode");
    }
}

#[test]
fn missing_file_resilience_in_binder() {
    // Task 1.5: missing files should be flagged in binder but not prevent opening the project (R1.7).
    let dir = std::env::temp_dir().join("pstar-missing-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Create a project with three docs, but only two files exist.
    let doc1 = dir.join("exists1.md");
    let doc2 = dir.join("missing.md"); // This file will NOT be created
    let doc3 = dir.join("exists2.md");

    std::fs::write(&doc1, "File 1 content").unwrap();
    // doc2 intentionally not created
    std::fs::write(&doc3, "File 3 content").unwrap();

    let manifest_path = dir.join("test.pstarproj");
    let project = Project {
        manifest_path: manifest_path.clone(),
        manifest: ProjectManifest {
            name: "Test Project".to_string(),
            docs: vec![
                DocEntry {
                    path: doc1.clone(),
                    title: "Existing Doc 1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                DocEntry {
                    path: doc2.clone(),
                    title: "Missing Doc".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                DocEntry {
                    path: doc3.clone(),
                    title: "Existing Doc 3".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
            ],
            separator: crate::project::Separator::PageBreak,
        },
    };
    project.save().unwrap();

    // Load the project - should succeed despite missing file (R1.7).
    let loaded_project = Project::load(&manifest_path).unwrap();
    assert_eq!(loaded_project.manifest.docs.len(), 3);

    // Verify doc_exists correctly identifies missing file.
    assert!(loaded_project.doc_exists(0), "Doc 1 should exist");
    assert!(!loaded_project.doc_exists(1), "Doc 2 should be missing");
    assert!(loaded_project.doc_exists(2), "Doc 3 should exist");

    // Verify word count returns None for missing file.
    assert!(
        loaded_project.doc_word_count(0).is_some(),
        "Word count should work for existing files"
    );
    assert!(
        loaded_project.doc_word_count(1).is_none(),
        "Word count should return None for missing file"
    );
    assert!(
        loaded_project.doc_word_count(2).is_some(),
        "Word count should work for existing files"
    );

    // Create an App with this project and open the binder.
    let mut app = App::new(None).unwrap();
    app.project = Some(loaded_project);

    // Trigger binder toggle to build entries.
    app.execute(Cmd::BinderToggle);

    // Verify binder entries reflect the existence status.
    if let Mode::Binder { entries, .. } = &app.mode {
        assert_eq!(entries.len(), 3);
        assert!(entries[0].exists, "Entry 0 should exist");
        assert!(!entries[1].exists, "Entry 1 should be marked as missing");
        assert!(entries[2].exists, "Entry 2 should exist");

        assert!(entries[0].word_count.is_some());
        assert!(entries[1].word_count.is_none());
        assert!(entries[2].word_count.is_some());
    } else {
        panic!("Should be in binder mode");
    }

    // Simulate trying to open the missing file from the binder.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Existing Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Missing Doc".to_string(),
                word_count: None,
                exists: false,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Existing Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 1, // Select the missing file
    };

    // Try to open it by simulating Enter key.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.handle_binder_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    // Should show error message and stay in binder mode.
    assert!(
        app.status_msg.as_ref().unwrap().contains("missing"),
        "Should show missing file error"
    );
    assert!(
        matches!(app.mode, Mode::Binder { .. }),
        "Should stay in binder mode"
    );

    // Verify we can open existing files.
    app.mode = Mode::Binder {
        entries: vec![
            BinderEntry {
                idx: 0,
                title: "Existing Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 1,
                title: "Missing Doc".to_string(),
                word_count: None,
                exists: false,
                synopsis: String::new(),
            },
            BinderEntry {
                idx: 2,
                title: "Existing Doc 3".to_string(),
                word_count: Some(3),
                exists: true,
                synopsis: String::new(),
            },
        ],
        selected: 0, // Select existing file
    };

    app.handle_binder_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    // Should successfully open and exit binder mode.
    assert!(
        matches!(app.mode, Mode::Normal),
        "Should exit binder after opening existing file"
    );
    assert!(
        app.status_msg.as_ref().unwrap().contains("Opened"),
        "Should show success message"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// --- Phase 7: snapshots, revisions, diff, restore (R4) -------------------

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(plain(KeyCode::Char(c)));
    }
}

/// An app on a real file, with every write-heavy subsystem pointed at a
/// temporary root instead of the writer's real metadata tree.
fn test_app(tag: &str, text: &str) -> (PathBuf, PathBuf, App) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("chapter.md");
    std::fs::write(&source, text).unwrap();

    let mut app = App::new(Some(source.clone())).unwrap();
    app.splash = false;
    app.snapshot_root = Some(dir.join("snapshots"));
    // Keep the other write-heavy subsystems out of the writer's real dirs.
    app.backup_root = Some(dir.join("recovery"));
    app.backup_depth = 0;
    app.recovery_journals[0] = Journal::in_root(&dir.join("recovery"), Some(&source), "unused");
    (dir, source, app)
}

fn store_of(app: &App, source: &Path) -> SnapshotStore {
    SnapshotStore::for_file_in(app.snapshot_root.as_deref().unwrap(), source)
}

#[test]
fn snapshot_command_prompts_for_a_label_and_writes_the_text() {
    let (dir, source, mut app) = test_app("snap-manual", "Chapter One\n");

    app.execute(Cmd::Snapshot);
    assert!(
        matches!(
            app.mode,
            Mode::Input {
                action: InputAction::SnapshotLabel,
                ..
            }
        ),
        "^KN should prompt for a label"
    );
    typed(&mut app, "before the cut");
    app.handle_key(plain(KeyCode::Enter));

    assert!(matches!(app.mode, Mode::Normal));
    let store = store_of(&app, &source);
    assert_eq!(store.entries().len(), 1);
    let entry = &store.entries()[0];
    assert_eq!(entry.label.as_deref(), Some("before the cut"));
    assert!(
        !entry.auto,
        "a snapshot the writer asked for is not automatic"
    );
    assert_eq!(store.read_text(entry).unwrap(), "Chapter One\n");
    assert!(app.status_msg.as_ref().unwrap().contains("Snapshot"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_label_still_takes_the_snapshot() {
    let (dir, source, mut app) = test_app("snap-unlabelled", "text\n");

    app.execute(Cmd::Snapshot);
    // Enter on an empty prompt cancels every other command; here it means
    // "no label, take it anyway".
    app.handle_key(plain(KeyCode::Enter));

    let store = store_of(&app, &source);
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0].label, None);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn snapshotting_an_unsaved_buffer_is_refused_and_changes_nothing() {
    let mut app = App::new(None).unwrap();
    app.splash = false;
    app.snapshot_root = None;
    app.insert_text("unsaved words", EditKind::Other);

    app.execute(Cmd::Snapshot);

    assert!(
        matches!(app.mode, Mode::Normal),
        "no prompt without a path to key snapshots by"
    );
    assert!(
        app.status_msg
            .as_ref()
            .unwrap()
            .contains("Save the document")
    );
    assert_eq!(app.buf.rope.to_string(), "unsaved words");
}

#[test]
fn saving_takes_an_automatic_snapshot_and_retention_spares_manual_ones() {
    let (dir, source, mut app) = test_app("snap-on-save", "version one\n");
    app.snapshot_keep = 2;

    // A version the writer asked to keep, taken before any save.
    app.take_snapshot(Some("keep me"));

    for text in ["version two\n", "version three\n", "version four\n"] {
        let end = app.buf.len_chars();
        app.apply_edit(0, end, text, EditKind::Other, 0);
        app.execute(Cmd::Save);
        assert!(!app.buf.dirty, "save should succeed: {:?}", app.status_msg);
    }

    let store = store_of(&app, &source);
    let (manual, auto): (Vec<_>, Vec<_>) = store.entries().iter().partition(|e| !e.auto);
    assert_eq!(
        manual.len(),
        1,
        "R4.2: retention never prunes manual copies"
    );
    assert_eq!(manual[0].label.as_deref(), Some("keep me"));
    assert_eq!(store.read_text(manual[0]).unwrap(), "version one\n");
    assert_eq!(auto.len(), 2, "snapshot_keep = 2 automatic versions");
    assert_eq!(store.read_text(auto[1]).unwrap(), "version four\n");
    assert!(
        !app.status_msg.as_ref().unwrap().contains("snapshot failed"),
        "{:?}",
        app.status_msg
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn automatic_snapshots_can_be_turned_off_without_losing_existing_ones() {
    let (dir, source, mut app) = test_app("snap-disabled", "one\n");
    app.take_snapshot(Some("kept"));
    app.snapshot_keep = 0;

    let end = app.buf.len_chars();
    app.apply_edit(0, end, "two\n", EditKind::Other, 0);
    app.execute(Cmd::Save);

    let store = store_of(&app, &source);
    assert_eq!(store.entries().len(), 1, "no new automatic snapshot");
    assert_eq!(store.entries()[0].label.as_deref(), Some("kept"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn revisions_list_is_newest_first_and_diffs_against_the_current_draft() {
    let (dir, _source, mut app) = test_app("snap-revisions", "one\ntwo\n");
    app.take_snapshot(Some("older"));
    std::thread::sleep(Duration::from_millis(2));
    app.take_snapshot(Some("newer"));

    app.execute(Cmd::RevisionsList);
    let Mode::Revisions { entries, .. } = &app.mode else {
        panic!("expected the revisions list, got another mode");
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].label.as_deref(), Some("newer"));
    assert_eq!(entries[1].label.as_deref(), Some("older"));

    // Change the draft, then diff the selected version against it (R4.4).
    let end = app.buf.len_chars();
    app.apply_edit(0, end, "one\ntwo and a half\n", EditKind::Other, 0);
    app.execute(Cmd::RevisionsList); // toggles closed
    app.execute(Cmd::RevisionsList);
    app.handle_key(plain(KeyCode::Enter));

    let Mode::Diff { title, lines, .. } = &app.mode else {
        panic!("Enter should open the diff view, got {:?}", app.status_msg);
    };
    assert!(title.contains("current draft"), "{title}");
    assert!(title.contains("+1"), "{title}");
    assert!(title.contains("−1"), "{title}");
    assert!(
        lines
            .iter()
            .any(|l| l.tag == crate::diff::DiffTag::Insert && l.text == "two and a half")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn diffing_an_unchanged_version_says_so_and_keeps_the_list_open() {
    let (dir, _source, mut app) = test_app("snap-identical", "unchanged\n");
    app.take_snapshot(None);

    app.execute(Cmd::RevisionsList);
    app.handle_key(plain(KeyCode::Enter));

    assert!(
        matches!(app.mode, Mode::Revisions { .. }),
        "an empty diff should not look like a failure that closed the list"
    );
    assert!(app.status_msg.as_ref().unwrap().contains("identical"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn marking_a_version_diffs_two_snapshots_oldest_first() {
    let (dir, _source, mut app) = test_app("snap-two-way", "first\n");
    app.take_snapshot(Some("older"));
    std::thread::sleep(Duration::from_millis(2));
    let end = app.buf.len_chars();
    app.apply_edit(0, end, "second\n", EditKind::Other, 0);
    app.take_snapshot(Some("newer"));

    app.execute(Cmd::RevisionsList);
    // The list is newest-first: mark "newer", which advances onto "older",
    // then Enter. The diff must still read older → newer.
    app.handle_key(plain(KeyCode::Char(' ')));
    app.handle_key(plain(KeyCode::Enter));

    let Mode::Diff { title, lines, .. } = &app.mode else {
        panic!("expected a two-snapshot diff, got {:?}", app.status_msg);
    };
    assert!(!title.contains("current draft"), "{title}");
    let older_first = title.find("older").unwrap() < title.find("newer").unwrap();
    assert!(older_first, "diff should read chronologically: {title}");
    assert!(lines.iter().any(|l| l.text == "first"));
    assert!(lines.iter().any(|l| l.text == "second"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn restore_replaces_the_buffer_in_one_undo_step_and_is_reversible() {
    let (dir, source, mut app) = test_app("snap-restore", "the original chapter\n");
    app.take_snapshot(Some("original"));

    // Revise, in several separate edits, so a restore that merely undid the
    // last edit would be visibly wrong.
    let end = app.buf.len_chars();
    app.apply_edit(0, end, "a revision\n", EditKind::Other, 0);
    app.history.break_group();
    app.insert_text("and more\n", EditKind::Other);
    let revised = app.buf.rope.to_string();

    app.execute(Cmd::RevisionsList);
    app.handle_key(ctrl('r'));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.rope.to_string(), "the original chapter\n");
    assert!(app.buf.dirty, "a restore is unsaved work until it is saved");
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "the original chapter\n",
        "the file on disk is untouched by a restore"
    );
    assert_eq!(app.doc_stats.words, 3, "counts follow the restored text");

    // R4.5: exactly one undo step, and it puts the revision back verbatim.
    app.execute(Cmd::Undo);
    assert_eq!(app.buf.rope.to_string(), revised);

    // ...and the restore is reversible: with the undo chain broken by any
    // other command, undo undoes the undo (ADR-003's never-lose model), so
    // the restored version is reachable again.
    app.execute(Cmd::Right);
    app.execute(Cmd::Undo);
    assert_eq!(app.buf.rope.to_string(), "the original chapter\n");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn restore_from_the_diff_view_brings_back_the_older_version() {
    let (dir, _source, mut app) = test_app("snap-diff-restore", "snapshot text\n");
    app.take_snapshot(Some("mine"));
    let end = app.buf.len_chars();
    app.apply_edit(0, end, "draft text\n", EditKind::Other, 0);

    app.execute(Cmd::RevisionsList);
    app.handle_key(plain(KeyCode::Enter));
    assert!(matches!(app.mode, Mode::Diff { .. }));
    app.handle_key(ctrl('r'));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.rope.to_string(), "snapshot text\n");
    assert!(app.status_msg.as_ref().unwrap().contains("Restored"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_restore_leaves_the_buffer_alone() {
    let (dir, source, mut app) = test_app("snap-restore-fail", "working text\n");
    app.take_snapshot(Some("doomed"));
    // The snapshot file disappears between listing and restoring.
    let store = store_of(&app, &source);
    let entry = store.entries()[0].clone();
    std::fs::remove_file(store.path_of(&entry)).unwrap();

    app.restore_snapshot(&entry);

    assert_eq!(app.buf.rope.to_string(), "working text\n");
    assert!(!app.buf.dirty);
    assert!(app.status_msg.as_ref().unwrap().contains("Restore failed"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_diff_view_scrolls_and_closes_without_touching_the_document() {
    let (dir, _source, mut app) = test_app("snap-diff-scroll", "");
    let mut long = String::new();
    for i in 0..60 {
        long.push_str(&format!("line {i}\n"));
    }
    app.apply_edit(0, app.buf.len_chars(), &long, EditKind::Other, 0);
    app.take_snapshot(Some("long"));
    let revised = long
        .replace("line 5\n", "line five\n")
        .replace("line 50\n", "");
    app.apply_edit(0, app.buf.len_chars(), &revised, EditKind::Other, 0);

    app.execute(Cmd::RevisionsList);
    app.handle_key(plain(KeyCode::Enter));
    let Mode::Diff { scroll, .. } = &app.mode else {
        panic!("expected the diff view");
    };
    assert_eq!(*scroll, 0);

    app.handle_key(plain(KeyCode::Down));
    app.handle_key(plain(KeyCode::Down));
    let Mode::Diff { scroll, lines, .. } = &app.mode else {
        panic!("expected the diff view");
    };
    assert_eq!(*scroll, 2);
    let count = lines.len();

    // Scrolling can't run off either end.
    app.handle_key(plain(KeyCode::End));
    app.handle_key(plain(KeyCode::PageDown));
    let Mode::Diff { scroll, .. } = &app.mode else {
        panic!("expected the diff view");
    };
    assert_eq!(*scroll, count - 1);
    app.handle_key(plain(KeyCode::Home));
    app.handle_key(plain(KeyCode::Up));
    let Mode::Diff { scroll, .. } = &app.mode else {
        panic!("expected the diff view");
    };
    assert_eq!(*scroll, 0);

    app.handle_key(plain(KeyCode::Esc));
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.buf.rope.to_string(), revised, "viewing is not editing");

    let _ = std::fs::remove_dir_all(dir);
}

// --- Phase 8: sprints and focus mode (R3) ----------------------------------

#[test]
fn the_sprint_prompt_starts_a_countdown() {
    let (dir, _source, mut app) = test_app("sprint-start", "one two three\n");

    app.execute(Cmd::SprintStart);
    assert!(
        matches!(
            app.mode,
            Mode::Input {
                action: InputAction::SprintSpec,
                ..
            }
        ),
        "^OP should prompt for the sprint's terms"
    );
    typed(&mut app, "25/500");
    app.handle_key(plain(KeyCode::Enter));

    assert!(app.sprint.is_some());
    let msg = app.status_msg.clone().unwrap();
    assert!(msg.contains("Sprint started"), "{msg}");
    assert!(msg.contains("0/500"), "{msg}");
    assert!(msg.contains("⏱ 25:00"), "{msg}");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_nonsense_sprint_spec_reports_and_starts_nothing() {
    let (dir, _source, mut app) = test_app("sprint-bad-spec", "text\n");

    app.execute(Cmd::SprintStart);
    typed(&mut app, "soon");
    app.handle_key(plain(KeyCode::Enter));

    assert!(app.sprint.is_none());
    assert!(
        app.status_msg.as_ref().unwrap().contains("not a number"),
        "{:?}",
        app.status_msg
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_expired_sprint_reports_words_and_time_and_is_filed_in_history() {
    let (dir, _source, mut app) = test_app("sprint-expiry", "start\n");
    let started = Instant::now();
    app.execute(Cmd::SprintStart);
    typed(&mut app, "25");
    app.handle_key(plain(KeyCode::Enter));

    // Write during the sprint, including a `..` note that must not count
    // (R2.6 consistency: a sprint measures prose, like every other count).
    app.set_cursor(app.buf.len_chars());
    app.insert_text(
        "four more prose words\n.. a note to self\n",
        EditKind::Other,
    );

    // Not over yet.
    app.tick_sprint_at(started + Duration::from_secs(60));
    assert!(app.sprint.is_some());
    assert!(app.daily_history.sprints.is_empty());

    // The clock runs out.
    app.tick_sprint_at(started + Duration::from_secs(25 * 60 + 1));

    assert!(app.sprint.is_none(), "an expired sprint stops running");
    let msg = app.status_msg.clone().unwrap();
    assert!(msg.contains("Sprint done"), "{msg}");
    assert!(msg.contains("4 words"), "prose words only: {msg}");
    assert!(msg.contains("25:0"), "elapsed time reported: {msg}");
    assert!(
        msg.contains('✓'),
        "a timed sprint that ran out met its terms"
    );

    assert_eq!(app.daily_history.sprints.len(), 1, "R3.2: filed in history");
    let record = &app.daily_history.sprints[0];
    assert_eq!(record.words, 4);
    assert!(record.met_target);
    assert!(record.seconds >= 25 * 60);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn meeting_the_word_target_finishes_the_sprint_before_the_clock() {
    let (dir, _source, mut app) = test_app("sprint-target", "");
    let started = Instant::now();
    app.execute(Cmd::SprintStart);
    typed(&mut app, "60/5");
    app.handle_key(plain(KeyCode::Enter));

    app.insert_text("one two three four", EditKind::Other);
    app.tick_sprint_at(started + Duration::from_secs(30));
    assert!(app.sprint.is_some(), "four of five words is not done");

    app.insert_text(" five", EditKind::Other);
    app.tick_sprint_at(started + Duration::from_secs(31));

    assert!(app.sprint.is_none());
    assert_eq!(app.daily_history.sprints[0].words, 5);
    assert!(app.daily_history.sprints[0].met_target);
    assert!(app.daily_history.sprints[0].seconds < 60 * 60);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stopping_a_sprint_reports_it_without_filing_it() {
    let (dir, _source, mut app) = test_app("sprint-stop", "");
    app.execute(Cmd::SprintStart);
    typed(&mut app, "25/500");
    app.handle_key(plain(KeyCode::Enter));
    app.insert_text("a few words here", EditKind::Other);

    // The same chord stops it.
    app.execute(Cmd::SprintStart);

    assert!(app.sprint.is_none());
    let msg = app.status_msg.clone().unwrap();
    assert!(msg.contains("Sprint stopped"), "{msg}");
    assert!(msg.contains("4 words"), "{msg}");
    assert!(
        app.daily_history.sprints.is_empty(),
        "R3.2 files sprints that ended, not ones called off"
    );
    // ...and the prompt is available again for a fresh sprint.
    app.execute(Cmd::SprintStart);
    assert!(matches!(
        app.mode,
        Mode::Input {
            action: InputAction::SprintSpec,
            ..
        }
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn focus_mode_forces_help_level_zero_and_restores_it() {
    let (dir, _source, mut app) = test_app("focus-toggle", "text\n");
    app.help_level = 2;

    app.execute(Cmd::FocusMode);
    assert!(app.focus.is_some());
    assert_eq!(app.help_level, 0, "R3.3: chrome off means help level 0");
    assert!(app.status_msg.as_ref().unwrap().contains("Focus mode"));

    app.execute(Cmd::FocusMode);
    assert!(app.focus.is_none());
    assert_eq!(app.help_level, 2, "the writer's help level comes back");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sprints_and_focus_never_touch_the_text_or_the_file() {
    // R3.5: both are presentational. Run a whole sprint, toggle focus around it,
    // and the document must be byte-identical and still clean.
    let (dir, source, mut app) = test_app("presentational", "the manuscript\n");
    let before = app.buf.rope.to_string();
    let started = Instant::now();

    app.execute(Cmd::FocusMode);
    app.execute(Cmd::SprintStart);
    typed(&mut app, "1/10");
    app.handle_key(plain(KeyCode::Enter));
    app.tick_sprint_at(started + Duration::from_secs(61));
    app.execute(Cmd::FocusMode);

    assert!(app.sprint.is_none());
    assert_eq!(app.buf.rope.to_string(), before);
    assert!(!app.buf.dirty, "no edit means no dirty buffer");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), before);
    assert_eq!(app.cursor, 0, "neither command moves the cursor");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_sprint_survives_focus_mode_and_keeps_counting() {
    let (dir, _source, mut app) = test_app("sprint-in-focus", "");
    let started = Instant::now();
    app.execute(Cmd::SprintStart);
    typed(&mut app, "25/500");
    app.handle_key(plain(KeyCode::Enter));
    app.execute(Cmd::FocusMode);

    app.insert_text("words written while focused", EditKind::Other);
    let sprint = app.sprint.clone().unwrap();
    assert_eq!(sprint.words_written(app.doc_stats.words), 4);
    assert!(
        sprint
            .chip(started + Duration::from_secs(60), app.doc_stats.words)
            .contains("4/500")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_sprint_report_survives_the_keystroke_that_ends_the_sprint() {
    // A sprint usually completes mid-word, so the next letter typed would clear
    // an ordinary status message and the writer would never see the report
    // (R3.2). The banner outlives it.
    let (dir, _source, mut app) = test_app("sprint-banner", "");
    let started = Instant::now();
    app.execute(Cmd::SprintStart);
    typed(&mut app, "60/2");
    app.handle_key(plain(KeyCode::Enter));

    app.insert_text("one two", EditKind::Other);
    app.tick_sprint_at(started + Duration::from_secs(5));
    assert!(app.status_msg.as_ref().unwrap().contains("Sprint done"));

    // Keep typing: the transient status message goes, the report stays.
    app.handle_key(plain(KeyCode::Char('!')));
    assert!(app.status_msg.is_none());
    let banner = app
        .sprint_banner()
        .expect("report should outlive a keystroke");
    assert!(banner.contains("Sprint done"), "{banner}");
    assert!(banner.contains("2 words"), "{banner}");

    // Starting the next sprint clears the last one's report.
    app.execute(Cmd::SprintStart);
    assert!(app.sprint_banner().is_none());

    let _ = std::fs::remove_dir_all(dir);
}

// --- Phase 9: notes, synopsis, and note documents (R5) ----------------------

/// A test app whose sidecar root is a temporary directory.
fn notes_app(tag: &str, text: &str) -> (PathBuf, PathBuf, App) {
    let (dir, source, mut app) = test_app(tag, text);
    app.meta_root = Some(dir.join("meta"));
    (dir, source, app)
}

#[test]
fn the_synopsis_prompt_is_prefilled_and_saves_to_the_sidecar() {
    let (dir, source, mut app) = notes_app("synopsis", "Chapter One\n");
    let root = app.meta_root.clone().unwrap();

    app.execute(Cmd::EditSynopsis);
    match &app.mode {
        Mode::Input { value, action, .. } => {
            assert_eq!(*action, InputAction::Synopsis);
            assert_eq!(value, "", "nothing written yet");
        }
        _ => panic!("^PI should prompt for a synopsis"),
    }
    typed(&mut app, "Marcus finds the knife.");
    app.handle_key(plain(KeyCode::Enter));

    assert_eq!(
        crate::meta::synopsis(&root, &source),
        "Marcus finds the knife."
    );
    assert!(app.status_msg.as_ref().unwrap().contains("Synopsis saved"));
    // R5.1: never written beside the manuscript. (That the production root is
    // the platform metadata directory is asserted in meta.rs's own tests.)
    assert_ne!(
        crate::meta::meta_path(&root, &source).parent(),
        source.parent()
    );

    // Re-prompting offers the current text for editing rather than a blank line.
    app.execute(Cmd::EditSynopsis);
    match &app.mode {
        Mode::Input { value, .. } => assert_eq!(value, "Marcus finds the knife."),
        _ => panic!("expected the synopsis prompt"),
    }
    // Enter accepts the pre-filled line unchanged.
    app.handle_key(plain(KeyCode::Enter));
    assert_eq!(
        crate::meta::synopsis(&root, &source),
        "Marcus finds the knife."
    );

    // Emptying the line and pressing Enter clears it — for this prompt an empty
    // answer is an answer, not a cancel.
    app.execute(Cmd::EditSynopsis);
    for _ in 0.."Marcus finds the knife.".len() {
        app.handle_key(plain(KeyCode::Backspace));
    }
    app.handle_key(plain(KeyCode::Enter));
    assert_eq!(crate::meta::synopsis(&root, &source), "");
    assert!(app.status_msg.as_ref().unwrap().contains("cleared"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn metadata_needs_a_saved_document() {
    let mut app = App::new(None).unwrap();
    app.splash = false;
    app.insert_text("unsaved", EditKind::Other);

    app.execute(Cmd::EditSynopsis);
    assert!(matches!(app.mode, Mode::Normal), "no prompt without a path");
    assert!(
        app.status_msg
            .as_ref()
            .unwrap()
            .contains("Save the document"),
        "{:?}",
        app.status_msg
    );

    app.execute(Cmd::OpenNotes);
    assert_eq!(
        app.panes.len(),
        1,
        "no split without somewhere to key notes"
    );
}

#[test]
fn notes_open_beside_the_manuscript_as_an_ordinary_document() {
    let (dir, source, mut app) = notes_app("notes-split", "the manuscript\n");
    let root = app.meta_root.clone().unwrap();

    app.execute(Cmd::OpenNotes);

    assert_eq!(app.panes.len(), 2, "R5.4: notes open in a split");
    assert_eq!(app.active, 1, "focus follows the notes");
    assert_eq!(
        app.buf.path.as_deref(),
        Some(crate::meta::notes_path(&root, &source).as_path())
    );
    assert_eq!(app.buf.rope.to_string(), "", "a fresh notes file is empty");

    // They are a real document: type, save, and the sidecar is on disk with the
    // ordinary atomic-save path (R5.6 comes free with that).
    app.insert_text("Marcus: left-handed.\n", EditKind::Other);
    app.execute(Cmd::Save);
    assert!(!app.buf.dirty, "{:?}", app.status_msg);
    assert_eq!(
        std::fs::read_to_string(crate::meta::notes_path(&root, &source)).unwrap(),
        "Marcus: left-handed.\n"
    );
    // The manuscript itself is untouched.
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "the manuscript\n"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opening_beside_never_discards_unsaved_work_in_the_other_window() {
    let (dir, _source, mut app) = notes_app("notes-clobber", "manuscript\n");
    app.execute(Cmd::OpenNotes);
    assert_eq!(app.panes.len(), 2);

    // Leave unsaved work in the notes pane, go back to the manuscript, and ask
    // for the notes again — the pane holding unsaved text must not be replaced.
    app.insert_text("unsaved research", EditKind::Other);
    app.active = 0;
    app.execute(Cmd::OpenNotes);

    assert_eq!(app.active, 0, "focus stays put when the command refuses");
    assert_eq!(app.panes[1].buf.rope.to_string(), "unsaved research");
    assert!(
        app.status_msg.as_ref().unwrap().contains("unsaved changes"),
        "{:?}",
        app.status_msg
    );

    let _ = std::fs::remove_dir_all(dir);
}

/// A project with a chapter and a character sheet, plus an app holding it.
fn project_with_note(tag: &str) -> (PathBuf, App) {
    let (dir, _source, mut app) = notes_app(tag, "chapter text\n");
    let chapter = dir.join("chapter1.md");
    let characters = dir.join("characters.md");
    std::fs::write(&chapter, "Chapter one text.\n").unwrap();
    std::fs::write(&characters, "Marcus: carries a knife.\n").unwrap();
    let manifest_path = dir.join("book.pstarproj");
    let project = Project {
        manifest_path: manifest_path.clone(),
        manifest: ProjectManifest {
            name: String::from("Book"),
            docs: vec![
                DocEntry {
                    path: chapter,
                    title: String::from("Chapter One"),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                DocEntry {
                    path: characters,
                    title: String::from("Characters"),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
            ],
            separator: crate::project::Separator::None,
        },
    };
    project.save().unwrap();
    app.project = Some(Project::load(&manifest_path).unwrap());
    (dir, app)
}

#[test]
fn marking_a_binder_document_as_a_note_persists_and_drops_it_from_compile() {
    let (dir, mut app) = project_with_note("mark-note");
    app.execute(Cmd::BinderToggle);
    // Select the character sheet.
    app.handle_key(plain(KeyCode::Down));

    app.execute(Cmd::ToggleDocRole);

    assert!(app.project.as_ref().unwrap().doc_is_note(1));
    let compiled = app.project.as_ref().unwrap().compile();
    assert!(compiled.text.contains("Chapter one text."));
    assert!(!compiled.text.contains("Marcus"), "{:?}", compiled.text);
    assert!(app.status_msg.as_ref().unwrap().contains("note"));
    // Persisted, and the binder is still open on the same row.
    let manifest_path = app.project.as_ref().unwrap().manifest_path.clone();
    assert!(Project::load(&manifest_path).unwrap().doc_is_note(1));
    match &app.mode {
        Mode::Binder { selected, .. } => assert_eq!(*selected, 1),
        _ => panic!("the binder should stay open"),
    }

    // And it is reversible.
    app.execute(Cmd::ToggleDocRole);
    assert!(!app.project.as_ref().unwrap().doc_is_note(1));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_binder_opens_the_selected_document_in_a_split() {
    let (dir, mut app) = project_with_note("binder-split");
    app.execute(Cmd::BinderToggle);
    app.handle_key(plain(KeyCode::Down));

    app.execute(Cmd::BinderOpenSplit);

    assert_eq!(app.panes.len(), 2, "R5.4");
    assert!(
        matches!(app.mode, Mode::Normal),
        "the binder closes behind it"
    );
    assert_eq!(app.buf.rope.to_string(), "Marcus: carries a knife.\n");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn binder_rows_carry_each_documents_synopsis() {
    let (dir, mut app) = project_with_note("binder-synopsis");
    let root = app.meta_root.clone().unwrap();
    let chapter = app.project.as_ref().unwrap().manifest.docs[0].path.clone();
    crate::meta::set_synopsis(&root, &chapter, "Marcus finds the knife.").unwrap();

    app.execute(Cmd::BinderToggle);

    match &app.mode {
        Mode::Binder { entries, .. } => {
            assert_eq!(entries[0].synopsis, "Marcus finds the knife.");
            assert_eq!(entries[1].synopsis, "", "no sidecar, no secondary line");
        }
        _ => panic!("expected the binder"),
    }

    let _ = std::fs::remove_dir_all(dir);
}

// --- Phase 10: editorial annotations (R9) -----------------------------------

fn comment_text(app: &App, i: usize) -> &str {
    app.annotations[i].text.as_str()
}

#[test]
fn a_comment_attaches_to_the_marked_block_and_persists() {
    let (dir, source, mut app) = notes_app("annotate-block", "The knife was on the table.\n");
    let root = app.meta_root.clone().unwrap();

    // Mark "knife" (chars 4..9) and comment on it.
    app.blocks.begin = Some(4);
    app.blocks.end = Some(9);
    app.execute(Cmd::Annotate);
    match &app.mode {
        Mode::Input {
            label,
            value,
            action,
        } => {
            assert_eq!(*action, InputAction::AnnotationText);
            assert!(label.contains("marked block"), "{label}");
            assert_eq!(value, "");
        }
        _ => panic!("^PC should prompt for the comment"),
    }
    typed(&mut app, "whose knife?");
    app.handle_key(plain(KeyCode::Enter));

    assert_eq!(app.annotations.len(), 1);
    assert_eq!((app.annotations[0].anchor, app.annotations[0].len), (4, 5));
    assert_eq!(comment_text(&app, 0), "whose knife?");
    // Written to the sidecar, beside the synopsis rather than in the manuscript.
    assert_eq!(
        crate::meta::annotations(&root, &source)[0].text,
        "whose knife?"
    );
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "The knife was on the table.\n",
        "R9.1: the comment never touches the prose"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_comment_without_a_block_attaches_to_the_cursor() {
    let (dir, _source, mut app) = notes_app("annotate-point", "some prose here\n");
    app.set_cursor(5);

    app.execute(Cmd::Annotate);
    typed(&mut app, "check this");
    app.handle_key(plain(KeyCode::Enter));

    assert_eq!((app.annotations[0].anchor, app.annotations[0].len), (5, 0));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_comment_under_the_cursor_is_shown_and_editable_in_place() {
    let (dir, _source, mut app) = notes_app("annotate-edit", "The knife was on the table.\n");
    app.blocks.begin = Some(4);
    app.blocks.end = Some(9);
    app.execute(Cmd::Annotate);
    typed(&mut app, "whose knife?");
    app.handle_key(plain(KeyCode::Enter));

    // Inside the span, ^PC edits that comment rather than adding another.
    app.set_cursor(6);
    assert_eq!(app.annotation_under_cursor(), Some("whose knife?"));
    app.execute(Cmd::Annotate);
    match &app.mode {
        Mode::Input { value, label, .. } => {
            assert_eq!(value, "whose knife?", "pre-filled for editing");
            assert!(label.contains("delete"), "{label}");
        }
        _ => panic!("expected the comment prompt"),
    }
    typed(&mut app, " (Marcus's)");
    app.handle_key(plain(KeyCode::Enter));
    assert_eq!(app.annotations.len(), 1, "edited, not duplicated");
    assert_eq!(comment_text(&app, 0), "whose knife? (Marcus's)");

    // Emptying it is the one way to delete a comment.
    app.execute(Cmd::Annotate);
    for _ in 0..comment_text(&app, 0).len() {
        app.handle_key(plain(KeyCode::Backspace));
    }
    app.handle_key(plain(KeyCode::Enter));
    assert!(app.annotations.is_empty());
    assert!(app.status_msg.as_ref().unwrap().contains("deleted"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn comment_anchors_follow_the_text_through_real_edits() {
    // R9.5: the same adjustment that keeps blocks and bookmarks attached.
    let (dir, _source, mut app) = notes_app("annotate-adjust", "The knife was on the table.\n");
    app.blocks.begin = Some(4);
    app.blocks.end = Some(9);
    app.execute(Cmd::Annotate);
    typed(&mut app, "whose knife?");
    app.handle_key(plain(KeyCode::Enter));
    app.blocks = Default::default();

    // Insert ahead of the anchored word.
    app.set_cursor(0);
    app.insert_text("Yesterday: ", EditKind::Other);
    assert_eq!((app.annotations[0].anchor, app.annotations[0].len), (15, 5));
    let text = app.buf.rope.to_string();
    assert_eq!(&text[15..20], "knife", "still on the word it was about");

    // Delete behind it.
    app.delete_range(0, 11, false);
    assert_eq!((app.annotations[0].anchor, app.annotations[0].len), (4, 5));

    // An edit elsewhere leaves it alone, and a bookmark on the same spot agrees.
    app.bookmarks[0] = Some(4);
    app.set_cursor(app.buf.len_chars());
    app.insert_text("\nA new line.\n", EditKind::Other);
    assert_eq!(app.annotations[0].anchor, 4);
    assert_eq!(app.bookmarks[0], Some(4), "annotations move like marks do");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn deleting_the_annotated_text_orphans_the_comment_and_keeps_it() {
    // R9.6/C3.
    let (dir, source, mut app) = notes_app("annotate-orphan", "The knife was on the table.\n");
    let root = app.meta_root.clone().unwrap();
    app.blocks.begin = Some(4);
    app.blocks.end = Some(9);
    app.execute(Cmd::Annotate);
    typed(&mut app, "whose knife?");
    app.handle_key(plain(KeyCode::Enter));
    app.blocks = Default::default();

    app.delete_range(0, app.buf.len_chars(), false);

    assert_eq!(app.annotations.len(), 1, "the comment survives its text");
    assert!(app.annotations[0].orphaned);
    assert_eq!(comment_text(&app, 0), "whose knife?");
    assert_eq!(app.annotation_under_cursor(), None, "nothing to point at");

    // And the orphan is written back, so it's still there tomorrow.
    app.execute(Cmd::Save);
    let stored = crate::meta::annotations(&root, &source);
    assert_eq!(stored.len(), 1);
    assert!(stored[0].orphaned);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn comments_come_back_when_the_document_is_reopened() {
    let (dir, source, mut app) = notes_app("annotate-reload", "The knife was on the table.\n");
    let root = app.meta_root.clone().unwrap();
    crate::meta::set_annotations(
        &root,
        &source,
        &[crate::meta::Annotation::new(
            4,
            5,
            String::from("whose knife?"),
        )],
    )
    .unwrap();

    // Opening the document into a pane loads its comments.
    let mut reopened = App::new(Some(source.clone())).unwrap();
    reopened.splash = false;
    reopened.meta_root = Some(root.clone());
    reopened.load_annotations(0);

    assert_eq!(reopened.annotations.len(), 1);
    reopened.set_cursor(6);
    assert_eq!(reopened.annotation_under_cursor(), Some("whose knife?"));

    // ...and a pane that adopts a *different* document doesn't inherit them.
    let other = dir.join("other.md");
    std::fs::write(&other, "unrelated\n").unwrap();
    reopened.switch_active_pane(other).unwrap();
    assert!(
        reopened.annotations.is_empty(),
        "comments belong to their document"
    );

    let _ = std::fs::remove_dir_all(dir);
    let _ = app.flush_annotations();
}

#[test]
fn navigation_walks_comments_and_the_list_jumps_to_them() {
    let (dir, source, mut app) = notes_app("annotate-nav", "one two three four five six\n");
    let root = app.meta_root.clone().unwrap();
    crate::meta::set_annotations(
        &root,
        &source,
        &[
            crate::meta::Annotation::new(4, 3, String::from("about two")),
            crate::meta::Annotation::new(14, 4, String::from("about four")),
            {
                let mut orphan = crate::meta::Annotation::new(26, 0, String::from("gone"));
                orphan.orphaned = true;
                orphan
            },
        ],
    )
    .unwrap();
    app.load_annotations(0);

    app.set_cursor(0);
    app.execute(Cmd::NextAnnotation);
    assert_eq!(app.cursor, 4);
    app.execute(Cmd::NextAnnotation);
    assert_eq!(app.cursor, 14);
    app.execute(Cmd::NextAnnotation);
    assert_eq!(app.cursor, 14, "orphans have nowhere to jump to");
    assert!(app.status_msg.as_ref().unwrap().contains("No further"));
    app.execute(Cmd::PrevAnnotation);
    assert_eq!(app.cursor, 4);

    // The list shows all three, orphans last, and Enter goes to one.
    app.execute(Cmd::AnnotationList);
    match &app.mode {
        Mode::Annotations { entries, .. } => {
            assert_eq!(entries.len(), 3);
            assert!(entries[2].orphaned, "orphans sort to the end");
        }
        _ => panic!("^PL should open the comment list"),
    }
    app.handle_key(plain(KeyCode::Down));
    app.handle_key(plain(KeyCode::Enter));
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.cursor, 14);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn comments_never_reach_an_export() {
    // R9.3. They can't: the text lives in the sidecar, not the document. This
    // guards that no future path smuggles it into the compiled prose.
    let (dir, source, mut app) = notes_app("annotate-export", "The knife was on the table.\n");
    let root = app.meta_root.clone().unwrap();
    crate::meta::set_annotations(
        &root,
        &source,
        &[crate::meta::Annotation::new(
            4,
            5,
            String::from("CUT THIS SCENE"),
        )],
    )
    .unwrap();
    app.load_annotations(0);

    for (cmd, name) in [
        (Cmd::ExportClean, "clean.txt"),
        (Cmd::ExportManuscript, "ms.rtf"),
        (Cmd::ExportHtml, "book.html"),
        (Cmd::ExportDocx, "book.docx"),
        (Cmd::ExportEpub, "book.epub"),
    ] {
        let out = dir.join(name);
        app.execute(cmd);
        typed(&mut app, out.to_str().unwrap());
        app.handle_key(plain(KeyCode::Enter));

        let bytes = std::fs::read(&out).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            !bytes.windows(14).any(|w| w == b"CUT THIS SCENE"),
            "{name} leaked an editorial comment"
        );
        // Sanity: the prose itself did make it out (except into zip containers,
        // where it is compressed away from a naive search).
        if !name.ends_with("docx") && !name.ends_with("epub") {
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("knife"), "{name} lost the prose");
        }
    }

    let _ = std::fs::remove_dir_all(dir);
}

// --- Phase 11: style & readability (R8) --------------------------------------

#[test]
fn style_checking_is_off_by_default_and_toggles() {
    // R8.1: optional. A fresh install doesn't open by arguing with the writer.
    let mut app = App::new(None).unwrap();
    app.splash = false;
    assert!(!app.style_enabled);

    app.execute(Cmd::ToggleStyle);
    assert!(app.style_enabled);
    assert!(
        app.status_msg
            .as_ref()
            .unwrap()
            .contains("Style checking on")
    );

    app.execute(Cmd::ToggleStyle);
    assert!(!app.style_enabled);
    assert!(app.status_msg.as_ref().unwrap().contains("off"));
}

#[test]
fn next_style_issue_walks_the_document_like_next_misspelling() {
    // R8.3.
    let (dir, _source, mut app) = test_app(
        "style-next",
        "He took the knife.\nHe walked quietly away.\nThe door was closed.\n",
    );

    // Off by default, the command says so rather than doing nothing.
    app.execute(Cmd::NextStyleIssue);
    assert!(
        app.status_msg.as_ref().unwrap().contains("^OY"),
        "{:?}",
        app.status_msg
    );
    assert_eq!(app.cursor, 0);

    app.execute(Cmd::ToggleStyle);
    app.set_cursor(0);
    app.execute(Cmd::NextStyleIssue);
    let first = app.cursor;
    assert!(
        first > 18,
        "the first issue is on the second line, got {first}"
    );
    assert!(
        app.status_msg.as_ref().unwrap().contains("adverb"),
        "{:?}",
        app.status_msg
    );

    app.execute(Cmd::NextStyleIssue);
    assert!(app.cursor > first, "it advances rather than re-reporting");
    assert!(
        app.status_msg.as_ref().unwrap().contains("passive"),
        "{:?}",
        app.status_msg
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_clean_document_reports_no_style_issues() {
    let (dir, _source, mut app) = test_app("style-clean", "He took the knife. She left.\n");
    app.execute(Cmd::ToggleStyle);

    app.execute(Cmd::NextStyleIssue);

    assert_eq!(app.cursor, 0);
    assert!(
        app.status_msg.as_ref().unwrap().contains("No style issues"),
        "{:?}",
        app.status_msg
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_cursor_reports_the_style_issue_it_is_sitting_on() {
    let (dir, _source, mut app) = test_app("style-cursor", "He walked quietly away.\n");
    app.execute(Cmd::ToggleStyle);

    app.set_cursor(12); // inside "quietly"
    assert_eq!(app.style_issue_at_cursor(), Some("-ly adverb"));
    app.set_cursor(0); // on "He"
    assert_eq!(app.style_issue_at_cursor(), None);

    // Nothing is reported while the checker is off.
    app.execute(Cmd::ToggleStyle);
    app.set_cursor(12);
    assert_eq!(app.style_issue_at_cursor(), None);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn readability_and_overused_words_are_computed_when_the_overlay_opens() {
    // R8.4/R8.5 on demand — and *only* on demand: computing this per frame on a
    // book-length document would land in the draw path.
    let (dir, _source, mut app) = test_app(
        "style-report",
        "The knife was on the table. The knife was Marcus's knife. He left.\n",
    );
    assert!(app.style_report.is_none());

    app.execute(Cmd::StatsOverlay);

    let report = app
        .style_report
        .clone()
        .expect("the overlay computes figures");
    assert!(!report.selection, "the whole document by default");
    assert_eq!(report.readability.sentences, 3);
    // A Flesch–Kincaid grade is legitimately near zero (or negative) for short
    // simple sentences, so the figure to assert is that it computed at all.
    assert!(report.readability.grade.is_finite());
    assert_eq!(report.readability.words, 13);
    assert_eq!(report.overused[0], (String::from("knife"), 3));

    // Closing it drops the snapshot rather than leaving stale figures behind.
    app.execute(Cmd::StatsOverlay);
    assert!(app.style_report.is_none());

    // With a block marked, the figures describe the selection (R8.4).
    app.blocks.begin = Some(0);
    app.blocks.end = Some(27);
    app.execute(Cmd::StatsOverlay);
    let report = app.style_report.clone().unwrap();
    assert!(report.selection);
    assert_eq!(report.readability.sentences, 1);

    let _ = std::fs::remove_dir_all(dir);
}
