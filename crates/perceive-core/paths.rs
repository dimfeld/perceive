use once_cell::sync::Lazy;

pub static PROJECT_DIRS: Lazy<directories::ProjectDirs> = Lazy::new(|| {
    let dirs = directories::ProjectDirs::from("", "dimfeld", "perceive-search")
        .expect("Could not get project directories");

    std::fs::create_dir_all(dirs.data_local_dir()).unwrap();

    dirs
});
