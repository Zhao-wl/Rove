//! #29：Markdown 预览的打包资源在构建时用 DEFLATE 压缩后嵌入 `rove.exe`（Mermaid 等合计约 4.3 MB，
//! 压缩后约 1.3 MB），请求时在内存里解压（`markdown::page::asset`）。woff2 字体本身已压缩，原样嵌入。
use std::{fs, io::Write, path::Path};

fn main() {
    let source = Path::new("assets/markdown");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("markdown");
    println!("cargo:rerun-if-changed=assets/markdown");
    compress_dir(source, source, &out);
}

fn compress_dir(root: &Path, dir: &Path, out: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "LICENSES") {
                continue;
            }
            compress_dir(root, &path, out);
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(root).unwrap();
        let target = out.join(relative).with_extension(format!(
            "{}.deflate",
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
        ));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&bytes).unwrap();
        fs::write(&target, encoder.finish().unwrap()).unwrap();
    }
}
