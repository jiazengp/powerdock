#[path = "build_support/locales.rs"]
mod locales;

fn main() {
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=build_support/locales.rs");
    let catalogs: Vec<_> = ["en-US", "zh-CN"]
        .into_iter()
        .map(|language| {
            let path = format!("locales/{language}.json");
            let source = std::fs::read_to_string(&path).expect("read locale catalog");
            let value =
                serde_json::from_str(&source).unwrap_or_else(|error| panic!("{path}: {error}"));
            (language, value)
        })
        .collect();
    locales::validate_catalogs(&catalogs).unwrap_or_else(|error| panic!("{error}"));
    let catalog = &catalogs[1].1;
    let mut code = String::from("fn status_key(value:&str)->Option<&'static str>{match value {\n");
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in catalog.as_object().unwrap() {
        if key.starts_with("Status") && seen.insert(value.as_str().unwrap()) {
            code.push_str(&format!(
                "{} => Some({}),\n",
                serde_json::to_string(value.as_str().unwrap()).unwrap(),
                serde_json::to_string(key).unwrap()
            ));
        }
    }
    code.push_str("_=>None}}\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("status_keys.rs"),
        code,
    )
    .unwrap();
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rerun-if-changed=assets/PowerDock.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/PowerDock.ico")
            .set("ProductName", "PowerDock")
            .set("FileDescription", "PowerDock")
            .set("OriginalFilename", "powerdock.exe")
            .compile()
            .expect("compile PowerDock Windows resources");
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg-bin=powerdock=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=powerdock=/MANIFESTINPUT:{root}/app.manifest");
    }
}
