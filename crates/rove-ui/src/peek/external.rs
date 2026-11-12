//! 外部符号片段的登记点（与 #42 元数据视图界面的接缝，issue #41 目标第 6 节）。
//!
//! 结果或定义落在外部符号（DLL，资源为 `rove-cs-metadata:<程序集|文档注释 ID>`）上时，速览左侧
//! 预览显示“片段”。本 issue 自带保底实现：只显示一行 `外部符号：<签名>`（签名来自 #38 的悬停
//! 数据）。#42 合并后调用 [`ExternalSnippets::register`] 注册真正的提供方（元数据视图正文 + 定位），
//! 速览的代码不需要改动。
//!
//! 提供方在**后台线程**调用，必须是 Send + Sync；返回 `None` 表示这个符号没有视图，退回保底。

use rove_base::{Cancellation, TextRange};
use rove_lang_cs::query::{Hover, Target};
use std::sync::{Arc, RwLock};

/// 预览里显示的片段：整段文本（当作 `language` 语言高亮）与要定位、高亮的区间。
#[derive(Clone, Debug)]
pub struct ExternalSnippet {
    pub text: Arc<str>,
    pub language: Arc<str>,
    pub focus: Option<TextRange>,
}

/// 外部符号片段提供方（#42 实现）。
pub trait ExternalSnippetProvider: Send + Sync {
    fn snippet(
        &self,
        target: &Target,
        hover: Option<&Hover>,
        cancel: &Cancellation,
    ) -> Option<ExternalSnippet>;
}

/// 片段从哪里来：注册的提供方，或保底的一行签名（文案由界面经字符串表生成）。
#[derive(Clone, Debug)]
pub enum Resolved {
    Provided(ExternalSnippet),
    Fallback { signature: Arc<str> },
}

/// 登记点：窗口装配时建一个，#42 向它注册提供方。
#[derive(Default)]
pub struct ExternalSnippets {
    provider: RwLock<Option<Arc<dyn ExternalSnippetProvider>>>,
}

impl ExternalSnippets {
    /// 注册（或替换）提供方。
    pub fn register(&self, provider: Arc<dyn ExternalSnippetProvider>) {
        *self.provider.write().unwrap_or_else(|e| e.into_inner()) = Some(provider);
    }

    /// 当前生效的是不是保底实现（PR 与测试据此说明状态）。
    pub fn is_fallback(&self) -> bool {
        self.provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }

    /// 后台调用：先问注册的提供方，没有结果时用保底签名。
    pub fn resolve(
        &self,
        target: &Target,
        hover: Option<&Hover>,
        signature: Arc<str>,
        cancel: &Cancellation,
    ) -> Resolved {
        let provider = self
            .provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match provider.and_then(|p| p.snippet(target, hover, cancel)) {
            Some(s) => Resolved::Provided(s),
            None => Resolved::Fallback { signature },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Position, Resource, Span};

    struct View;
    impl ExternalSnippetProvider for View {
        fn snippet(
            &self,
            _: &Target,
            _: Option<&Hover>,
            _: &Cancellation,
        ) -> Option<ExternalSnippet> {
            Some(ExternalSnippet {
                text: "public class Transform {}".into(),
                language: "csharp".into(),
                focus: None,
            })
        }
    }

    #[test]
    fn fallback_until_a_provider_registers() {
        let r = Span::new(Position::default(), Position::default()).unwrap();
        let target = Target {
            resource: Resource::Virtual(
                "rove-cs-metadata:UnityEngine|T:UnityEngine.Transform".into(),
            ),
            range: r.clone(),
            selection: r,
        };
        let registry = ExternalSnippets::default();
        assert!(registry.is_fallback());
        let c = Cancellation::default();
        assert!(matches!(
            registry.resolve(&target, None, "class Transform".into(), &c),
            Resolved::Fallback { signature } if &*signature == "class Transform"
        ));
        registry.register(Arc::new(View));
        assert!(!registry.is_fallback());
        assert!(matches!(
            registry.resolve(&target, None, "x".into(), &c),
            Resolved::Provided(s) if s.text.contains("Transform")
        ));
    }
}
