//! 验证 Unity 测试项目的约定（AGENTS.md §8）：
//!
//! - 环境变量未设置时**跳过而不是失败**（CI 上就是这样，`cargo test --workspace` 必须全绿）；
//! - 环境变量已设置时，指向的目录必须是一个 Unity 项目根目录（含 `Assets/`）。

use rove_test_support::{require_unity_project, UnityProject};

fn assert_unity_project(slot: UnityProject, path: std::path::PathBuf) {
    assert!(
        path.join("Assets").is_dir(),
        "{} 指向的目录不是 Unity 项目根目录（缺少 Assets/）：{}",
        slot.env_var(),
        path.display()
    );
    eprintln!("已运行：{} Unity 项目 = {}", slot.label(), path.display());
}

#[test]
fn medium_project_is_a_unity_project_when_configured() {
    let path = require_unity_project!(UnityProject::Medium);
    assert_unity_project(UnityProject::Medium, path);
}

#[test]
fn large_project_is_a_unity_project_when_configured() {
    let path = require_unity_project!(UnityProject::Large);
    assert_unity_project(UnityProject::Large, path);
}
