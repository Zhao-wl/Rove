fn main() {
    let source = "../../grammars/tree-sitter-shaderlab/src";
    println!("cargo:rerun-if-changed={source}");
    cc::Build::new()
        .include(source)
        .file(format!("{source}/parser.c"))
        .file(format!("{source}/scanner.c"))
        .compile("tree_sitter_shaderlab");
}
