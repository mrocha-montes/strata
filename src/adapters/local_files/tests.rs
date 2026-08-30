// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::ErrorKind,
    os::unix::ffi::{OsStrExt, OsStringExt},
    time::SystemTime,
};

use super::*;
use crate::model::Location;

#[test]
fn validation_accepts_readable_directories_and_rejects_files_and_missing_paths()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-location-test-{unique}"));
    let file = directory.join("file.txt");
    let missing = directory.join("missing");
    fs::create_dir(&directory)?;
    fs::write(&file, b"fixture")?;

    let source = LocalFileSource;
    assert_eq!(
        source.validate_location(&Location::local(&directory)),
        Ok(())
    );
    assert_eq!(
        source.validate_location(&Location::local(&file)),
        Err(LocationValidationError::NotDirectory)
    );
    assert_eq!(
        source.validate_location(&Location::local(&missing)),
        Err(LocationValidationError::Missing)
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn invalid_utf8_names_keep_their_native_bytes() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-native-name-test-{unique}"));
    fs::create_dir(&directory)?;
    let native_name = OsString::from_vec(b"invalid-\xff".to_vec());
    let path = directory.join(&native_name);
    fs::write(&path, b"fixture")?;

    let info = gio::File::for_path(&path).query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    )?;
    let entry = entry_from_info(Location::local(path.clone()), info);

    assert_eq!(entry.native_name.as_bytes(), native_name.as_bytes());
    assert_eq!(entry.location.native_path(), Some(path.as_path()));
    assert!(!entry.display_name.is_empty());

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn unmounted_network_shares_are_treated_as_directories() {
    let info = gio::FileInfo::new();
    info.set_file_type(gio::FileType::Mountable);
    info.set_is_symlink(false);
    info.set_name("share");
    info.set_display_name("share");

    let entry = entry_from_info(Location::uri("smb://host/share"), info);

    assert_eq!(entry.kind, EntryKind::Directory);
    assert!(entry.is_directory());
}

#[test]
fn symlink_targets_and_broken_links_are_distinguished() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-symlink-test-{unique}"));
    fs::create_dir(&directory)?;
    fs::create_dir(directory.join("directory"))?;
    fs::write(directory.join("file"), b"fixture")?;
    symlink("directory", directory.join("directory-link"))?;
    symlink("file", directory.join("file-link"))?;
    symlink("missing", directory.join("broken-link"))?;

    let kind = |name: &str| -> Result<EntryKind, glib::Error> {
        let path = directory.join(name);
        let info = gio::File::for_path(&path).query_info(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )?;
        Ok(entry_from_info(Location::local(path), info).kind)
    };

    assert_eq!(kind("directory-link")?, EntryKind::DirectorySymbolicLink);
    assert_eq!(kind("file-link")?, EntryKind::FileSymbolicLink);
    assert_eq!(kind("broken-link")?, EntryKind::SymbolicLink);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn coalescing_preserves_a_move_when_metadata_follows_it() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Upsert("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Remove("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Rescan));
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        map_validation_error(error),
        LocationValidationError::Inaccessible
    );
}
