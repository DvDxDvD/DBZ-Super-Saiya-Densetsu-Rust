use ssd_oracle::{FsSaveStore, MemorySaveStore, SaveStore};

#[test]
fn fs_save_store_round_trips_and_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FsSaveStore::new(dir.path().join("nested").join("saves"), "game").unwrap();

    assert!(store.load("sav").unwrap().is_none(), "missing file is Ok(None), not an error");

    store.persist("sav", b"first").unwrap();
    assert_eq!(store.load("sav").unwrap().as_deref(), Some(b"first".as_slice()));

    store.persist("sav", b"second, longer").unwrap();
    assert_eq!(store.load("sav").unwrap().as_deref(), Some(b"second, longer".as_slice()));

    assert!(dir.path().join("nested").join("saves").join("game.sav").is_file());
    assert!(
        !dir.path().join("nested").join("saves").join("game.sav.tmp").exists(),
        "temporary file must not be left behind"
    );
}

#[test]
fn fs_save_store_reports_unreadable_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("game.sav")).unwrap();
    let mut store = FsSaveStore::new(dir.path().to_path_buf(), "game").unwrap();
    assert!(store.load("sav").is_err(), "a read error must not be reported as 'no save'");
}

#[test]
fn memory_save_store_round_trips() {
    let mut store = MemorySaveStore::default();
    assert!(store.load("sav").unwrap().is_none());
    store.persist("sav", b"abc").unwrap();
    assert_eq!(store.load("sav").unwrap().as_deref(), Some(b"abc".as_slice()));
}
