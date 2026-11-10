//! Settings registration and access (#13). JSONC parsing/writing belongs to the implementation.
use rove_base::*;
use std::{marker::PhantomData, sync::Arc};
/// A registered VS Code or rove.* key. Overrides apply only in the absence of explicit user values.
/// The four compat/SETTINGS.md §3 exceptions are approved; all other differences use rove.* rules.
#[derive(Clone, Debug)]
pub struct Setting {
    pub default: Value,
    pub rove_default: Option<Value>,
    pub language_overridable: bool,
}
pub type SettingRegistry = Registry<Setting>;
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub language: Option<Arc<str>>,
}
#[derive(Clone, Debug)]
pub struct SettingsChange {
    pub revision: Version,
    pub keys: Arc<[Arc<str>]>,
}
/// Immutable typed conversion; mismatched type => InvalidInput, never silently coerced.
pub trait SettingValue: Sized {
    fn decode(value: &Value) -> Result<Self>;
}
impl SettingValue for bool {
    fn decode(v: &Value) -> Result<Self> {
        if let Value::Bool(x) = v {
            Ok(*x)
        } else {
            Err(Error::InvalidInput)
        }
    }
}
impl SettingValue for f64 {
    fn decode(v: &Value) -> Result<Self> {
        if let Value::Number(x) = v {
            if x.is_finite() {
                return Ok(*x);
            }
        }
        Err(Error::InvalidInput)
    }
}
impl SettingValue for String {
    fn decode(v: &Value) -> Result<Self> {
        if let Value::String(x) = v {
            Ok(x.to_string())
        } else {
            Err(Error::InvalidInput)
        }
    }
}
impl SettingValue for Value {
    fn decode(v: &Value) -> Result<Self> {
        Ok(v.clone())
    }
}
/// Typed consumer key. Registration remains explicit and duplicate-safe.
pub struct Key<T> {
    pub id: &'static str,
    marker: PhantomData<fn() -> T>,
}
impl<T: SettingValue> Key<T> {
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }
    pub fn read(&self, settings: &dyn Settings, scope: &Scope) -> Result<T> {
        T::decode(&settings.read(self.id, scope)?)
    }
}
/// Send + Sync cached read O(log keys), UI-safe/no file access. Language user override wins over
/// general user value, which wins over Rove fallback and then VS Code default. Invalid reload keeps
/// the last valid snapshot. Unknown registry key => NotFound. Write is explicit user intent only:
/// background JSONC patch of one key, preserves comments/order/format, backup first, fail on races.
/// Never write defaults on startup. Cancellation/Io/Stale propagate; no notification on failed write.
pub trait Settings: Send + Sync {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value>;
    fn write(&self, key: Arc<str>, scope: Scope, value: Value, cancel: Cancellation) -> Task<()>;
    fn changes(&self) -> &dyn Events<SettingsChange>;
}
/// Fixed defaults; no disk writes. #13 supplies overrides, validation, JSONC patches and events.
pub struct DefaultSettings(pub SettingRegistry);
impl Settings for DefaultSettings {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        let s = self.0.get(key).ok_or(Error::NotFound)?;
        let language = scope
            .language
            .as_deref()
            .filter(|_| s.language_overridable)
            .and_then(|lang| self.0.get(&format!("[{lang}]")));
        let member = |value: &'_ Value| match value {
            Value::Object(map) => map.get(key).cloned(),
            _ => None,
        };
        Ok(language
            .and_then(|entry| entry.rove_default.as_ref().and_then(member))
            .or_else(|| s.rove_default.clone())
            .or_else(|| language.and_then(|entry| member(&entry.default)))
            .unwrap_or_else(|| s.default.clone()))
    }
    fn write(&self, _: Arc<str>, _: Scope, _: Value, c: Cancellation) -> Task<()> {
        Box::pin(async move {
            c.check()?;
            Err(Error::Unsupported)
        })
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoding_consumer_reads_approved_default() {
        let mut r = SettingRegistry::default();
        r.register(
            "files.autoGuessEncoding",
            Setting {
                default: Value::Bool(false),
                rove_default: Some(Value::Bool(true)),
                language_overridable: true,
            },
        )
        .unwrap();
        let s = DefaultSettings(r);
        assert!(Key::<bool>::new("files.autoGuessEncoding")
            .read(
                &s,
                &Scope {
                    language: Some("csharp".into())
                }
            )
            .unwrap());
        assert!(Key::<String>::new("files.autoGuessEncoding")
            .read(&s, &Scope::default())
            .is_err());
        let _sub = s.changes().subscribe(Arc::new(|_| {}));
    }
}
