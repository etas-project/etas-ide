use std::{env, fs, path::PathBuf};

fn main() {
    let lock = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let manifest: toml::Table = fs::read_to_string(lock).unwrap().parse().unwrap();
    let packages = manifest["package"].as_array().unwrap();
    for (name, key) in [
        ("etas_frontend", "FRONTEND"),
        ("etas_core", "CORE"),
        ("etas_driver", "DRIVER"),
    ] {
        let matches = packages
            .iter()
            .filter(|package| package["name"].as_str() == Some(name))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "release must contain exactly one {name} identity"
        );
        let package = matches[0];
        let version = package["version"].as_str().unwrap();
        let source = package["source"]
            .as_str()
            .expect("compiler producer must be Cargo-locked git source");
        let (_, revision) = source
            .rsplit_once('#')
            .expect("compiler producer must have a git revision");
        println!("cargo:rustc-env=ETAS_{key}_VERSION={version}");
        println!("cargo:rustc-env=ETAS_{key}_REVISION={revision}");
    }
}
