use std::fs;
use std::path::Path;

fn main() {
    let dist_dir = Path::new("../dist");
    let dist_index = dist_dir.join("index.html");
    if !dist_index.exists() {
        let _ = fs::create_dir_all(dist_dir);
        let _ = fs::write(
            &dist_index,
            "<!doctype html><html><body><p>PackStudio frontend placeholder (run npm run build)</p></body></html>\n",
        );
    }

    tauri_build::build();
}
