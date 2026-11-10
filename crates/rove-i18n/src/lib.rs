//! Keyed Chinese UI strings. #67 adds English; no hard-coded user-visible prose in UI modules.
use rove_base::*;
use std::sync::Arc;
/// Registration uses stable keys. O(log keys), Send + Sync after assembly. Duplicate => Duplicate.
pub type StringTable = Registry<Arc<str>>;
/// Localized message carried by feedback and contributions; arguments are runtime data, not prose.
#[derive(Clone, Debug)]
pub struct Message {
    pub key: Arc<str>,
    pub args: Arc<[(Arc<str>, Arc<str>)]>,
}
impl Message {
    pub fn key(key: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            args: Arc::from([]),
        }
    }
}
/// Lookup is UI-safe, O(log keys + template bytes + placeholders * arguments + output bytes),
/// no I/O. `{name}` substitutes a named argument; `{{` and `}}` emit literal braces.
/// Missing keys return NotFound. Missing arguments, empty/nested placeholders and unmatched braces
/// return InvalidInput. Substituted values are literal and never recursively interpreted.
pub trait Strings: Send + Sync {
    fn text(&self, message: &Message) -> Result<String>;
}
impl Strings for StringTable {
    fn text(&self, message: &Message) -> Result<String> {
        let template = self.get(&message.key).ok_or(Error::NotFound)?;
        let mut out = String::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    out.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    out.push('}');
                }
                '{' => {
                    let mut key = String::new();
                    loop {
                        match chars.next() {
                            Some('}') if !key.is_empty() => break,
                            Some('{') | Some('}') | None => return Err(Error::InvalidInput),
                            Some(c) => key.push(c),
                        }
                    }
                    let (_, value) = message
                        .args
                        .iter()
                        .find(|(k, _)| k.as_ref() == key)
                        .ok_or(Error::InvalidInput)?;
                    out.push_str(value);
                }
                '}' => return Err(Error::InvalidInput),
                c => out.push(c),
            }
        }
        Ok(out)
    }
}
/// All UI string reads use this macro; failures must be handled at the presentation boundary.
#[macro_export]
macro_rules! tr {
    ($table:expr,$message:expr) => {
        $crate::Strings::text($table, $message)
    };
}
/// Minimal Chinese table, extended through registration by feature owners.
pub fn chinese() -> Result<StringTable> {
    let mut t = StringTable::default();
    t.register("common.loading", Arc::from("正在加载…"))?;
    t.register("common.retry", Arc::from("重试"))?;
    t.register("common.cancel", Arc::from("取消"))?;
    Ok(t)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn component_uses_chinese_key() {
        let mut t = chinese().unwrap();
        t.register("example.count", Arc::from("已找到 {count} 处"))
            .unwrap();
        let m = Message {
            key: "example.count".into(),
            args: Arc::from([("count".into(), "3".into())]),
        };
        assert_eq!(tr!(&t, &m).unwrap(), "已找到 3 处");
        assert!(tr!(&t, &Message::key("missing")).is_err());
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;
    #[test]
    fn escaped_braces_and_substitutions_are_literal() {
        let mut table = StringTable::default();
        table
            .register("example", Arc::from("中文 {{配置}}：{{{value}}} / {value}"))
            .unwrap();
        let message = Message {
            key: "example".into(),
            args: Arc::from([("value".into(), "😀{raw}".into())]),
        };
        assert_eq!(
            tr!(&table, &message).unwrap(),
            "中文 {配置}：{😀{raw}} / 😀{raw}"
        );
        table.register("braces", Arc::from("{{}}")).unwrap();
        assert_eq!(tr!(&table, &Message::key("braces")).unwrap(), "{}");
    }
    #[test]
    fn malformed_templates_and_missing_arguments_fail() {
        for template in ["{", "}", "{}", "{name", "{a{b}", "{missing}", "{{}"] {
            let mut table = StringTable::default();
            table.register("example", Arc::from(template)).unwrap();
            assert_eq!(
                tr!(&table, &Message::key("example")),
                Err(Error::InvalidInput),
                "{template}"
            );
        }
    }
}
