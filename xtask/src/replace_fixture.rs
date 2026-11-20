/// #48：10 万行、每 20 行一处 `TargetComponent`（共 5000 处），对应验收里的“替换 ≥ 5000 处”。
pub fn replace_source() -> String {
    let mut s = String::with_capacity(4_500_000);
    for i in 0..100_000u32 {
        if i % 20 == 0 {
            s.push_str(&format!(
                "        var target{i} = GetComponent<TargetComponent>(); // {i}\n"
            ));
        } else {
            s.push_str(&format!(
                "        int value{i} = Mathf.Max({i}, speed * deltaTime + offset);\n"
            ));
        }
    }
    s
}
