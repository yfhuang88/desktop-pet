use std::env;
use std::path::PathBuf;

fn main() {
    tauri_build::build();

    // 把面向用户的使用说明复制到编译产物同一目录(target/release/ 或 target/debug/)，
    // 这样每次编译完，说明文件都会自动出现在 exe 旁边，不需要手动复制。
    println!("cargo:rerun-if-changed=使用说明.txt");

    if let Ok(out_dir) = env::var("OUT_DIR") {
        // OUT_DIR 形如 target/<profile>/build/<pkg>-<hash>/out，向上 3 层就是 target/<profile>/
        let mut dest_dir = PathBuf::from(out_dir);
        for _ in 0..3 {
            dest_dir.pop();
        }

        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let src = manifest_dir.join("使用说明.txt");
        if src.exists() {
            let _ = std::fs::copy(&src, dest_dir.join("使用说明.txt"));
        }

        // 把默认的 5 张贴图复制成 target/<profile>/assets/ 下的"样例"，方便用户
        // 直接看到换皮肤要放哪些文件、长什么样。只在这个 assets 文件夹还不存在时才复制，
        // 一旦用户在里面放了自己的自定义图，以后重新编译不会覆盖掉他们的东西。
        let sample_assets_dest = dest_dir.join("assets");
        if !sample_assets_dest.exists() {
            let sample_assets_src = manifest_dir
                .parent()
                .expect("src-tauri 应该有上级目录")
                .join("src")
                .join("assets");
            if sample_assets_src.is_dir() {
                let _ = std::fs::create_dir_all(&sample_assets_dest);
                for name in ["idle.png", "walk1.png", "walk2.png", "walk3.png", "drag.png"] {
                    let file_src = sample_assets_src.join(name);
                    if file_src.exists() {
                        let _ = std::fs::copy(&file_src, sample_assets_dest.join(name));
                    }
                }
            }
        }
    }
}
