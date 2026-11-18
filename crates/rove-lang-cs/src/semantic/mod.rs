//! C# 语义 token 的产出（#59）：符号种类 + 修饰符 → VS Code 标准语义 token。
//!
//! 数据来源是 L2 语义索引里**每个出现位置绑定到的符号**（种类、容器、是否声明处、
//! 是否来自外部程序集），再由同一份文件的语法树补上索引里没有的声明细节
//! （类型是 class 还是 struct、`static` / `readonly` / `const` / `abstract` / `async`）。
//! 见 [`syntax`] 的模块文档与 `docs/ARCHITECTURE.md` §59。
//!
//! **不发明类型名**：类型名与修饰符名一律取 VS Code / LSP 的标准集合，否则用户主题里的
//! `semanticTokenColors` 规则对不上。

pub mod syntax;

#[cfg(test)]
mod tests;

/// C# 符号种类（映射表的左列）。比 Roslyn 的 `SymbolKind` 细：具名类型按 `TypeKind` 展开，
/// 常量字段与普通字段分开，局部函数与方法分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CsSymbolKind {
    Namespace,
    Class,
    Record,
    Struct,
    RecordStruct,
    Interface,
    Enum,
    Delegate,
    /// 具名类型，但种类不确定（例如来自外部 DLL、L1 里查不到）。
    Type,
    TypeParameter,
    Field,
    /// `const` 字段。
    Constant,
    EnumMember,
    Property,
    Event,
    Method,
    LocalFunction,
    Local,
    Parameter,
    /// LINQ 的范围变量（`from x in …` 的 `x`）。
    RangeVariable,
}

/// **C# 符号种类 → 标准语义 token 类型**。一行就是一条映射，不写分支逻辑；
/// `semantic::tests::each_row_of_the_kind_table_maps_to_its_token_type` 逐行断言。
///
/// 几条需要解释的选择：
/// - `record` / `record struct` 在 CLR 里就是 class / struct，取同样的 token 类型；
/// - C# 的 `delegate` 是一个类，标准集合里没有 `delegate`（VS Code 的 C# 扩展用的是它自己
///   登记的 `delegate` 类型），按「不发明类型名」的要求归到 `class`；
/// - `const` 字段取 `variable` 而不是 `field`：配上 `readonly` 修饰符后，回退表里的
///   `variable.readonly → variable.other.constant` 正好是 VS Code C# 扩展给常量用的 scope；
/// - 局部函数取 `function`，成员方法取 `method`（标准集合里这两个是分开的）。
pub const KIND_TABLE: &[(CsSymbolKind, &str)] = &[
    (CsSymbolKind::Namespace, "namespace"),
    (CsSymbolKind::Class, "class"),
    (CsSymbolKind::Record, "class"),
    (CsSymbolKind::Struct, "struct"),
    (CsSymbolKind::RecordStruct, "struct"),
    (CsSymbolKind::Interface, "interface"),
    (CsSymbolKind::Enum, "enum"),
    (CsSymbolKind::Delegate, "class"),
    (CsSymbolKind::Type, "type"),
    (CsSymbolKind::TypeParameter, "typeParameter"),
    (CsSymbolKind::Field, "field"),
    (CsSymbolKind::Constant, "variable"),
    (CsSymbolKind::EnumMember, "enumMember"),
    (CsSymbolKind::Property, "property"),
    (CsSymbolKind::Event, "event"),
    (CsSymbolKind::Method, "method"),
    (CsSymbolKind::LocalFunction, "function"),
    (CsSymbolKind::Local, "variable"),
    (CsSymbolKind::Parameter, "parameter"),
    (CsSymbolKind::RangeVariable, "variable"),
];

impl CsSymbolKind {
    /// 这个种类的标准 token 类型。
    pub fn token_type(self) -> &'static str {
        KIND_TABLE
            .iter()
            .find(|(k, _)| *k == self)
            .map(|(_, t)| *t)
            .expect("每个种类在 KIND_TABLE 里都有一行")
    }

    /// Roslyn `SymbolKind`（索引段 3 的 `kind` 字节）→ 本枚举的粗映射。
    /// 具名类型、字段与方法还要由调用方按容器与语法细化（见 [`syntax`]）。
    pub fn from_roslyn(kind: u8) -> Option<Self> {
        Some(match kind {
            5 => Self::Event,
            6 => Self::Field,
            8 => Self::Local,
            9 => Self::Method,
            11 => Self::Type,
            12 => Self::Namespace,
            13 => Self::Parameter,
            15 => Self::Property,
            16 => Self::RangeVariable,
            17 => Self::TypeParameter,
            _ => return None,
        })
    }

    pub fn is_type(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Record
                | Self::Struct
                | Self::RecordStruct
                | Self::Interface
                | Self::Enum
                | Self::Delegate
                | Self::Type
        )
    }
}

/// 标准语义 token 修饰符。`modification` 与 `documentation` 需要写访问与文档注释的判定，
/// 本 issue 不产出（留空比标错好）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// 这个位置就是符号的声明处。
    pub declaration: bool,
    pub static_: bool,
    pub abstract_: bool,
    pub deprecated: bool,
    pub async_: bool,
    pub readonly: bool,
    /// 来自本次索引之外的程序集（框架 / Unity / 第三方 DLL）。
    pub default_library: bool,
}

/// 修饰符名字的固定顺序（与 `tokenClassificationRegistry` 的登记顺序一致；
/// 匹配本身与顺序无关，固定顺序只是为了结果可比较）。
type ModifierProbe = (&'static str, fn(&Modifiers) -> bool);

const MODIFIER_NAMES: [ModifierProbe; 7] = [
    ("declaration", |m| m.declaration),
    ("static", |m| m.static_),
    ("abstract", |m| m.abstract_),
    ("deprecated", |m| m.deprecated),
    ("async", |m| m.async_),
    ("readonly", |m| m.readonly),
    ("defaultLibrary", |m| m.default_library),
];

impl Modifiers {
    pub fn names(&self) -> Vec<&'static str> {
        MODIFIER_NAMES
            .iter()
            .filter(|(_, f)| f(self))
            .map(|(n, _)| *n)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// 一个符号在某个位置的全部已知事实。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolFacts {
    pub kind: CsSymbolKind,
    pub modifiers: Modifiers,
}

impl SymbolFacts {
    pub fn new(kind: CsSymbolKind) -> Self {
        Self {
            kind,
            modifiers: Modifiers::default(),
        }
    }

    /// 最终的语义 token：类型名 + 修饰符名。
    pub fn classify(&self) -> (&'static str, Vec<&'static str>) {
        let mut modifiers = self.modifiers;
        // `const` 在 C# 里隐含 static + readonly（Roslyn 的 `IsStatic` 也是这么报的）。
        if self.kind == CsSymbolKind::Constant {
            modifiers.static_ = true;
            modifiers.readonly = true;
        }
        (self.kind.token_type(), modifiers.names())
    }
}
