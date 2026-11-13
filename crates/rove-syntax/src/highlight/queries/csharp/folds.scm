; 语法折叠区间（#118）。每个 capture 的节点是一个候选区间：节点首行折叠后仍可见，
; 末行（含）被隐藏；只占一行的节点在 Rust 侧丢弃。capture 名字不参与判定。
;
; 这里取的是带花括号的「体」而不是声明节点本身：Allman 风格（`class C` 换行再写 `{`）下
; 折叠箭头落在 `{` 行，与 #12 的缩进折叠给出的首行一致；K&R 风格下两者同在声明行。
; `#region` 折叠仍由语言配置的 folding.markers 产出（#12），这里不重复。
[
  (declaration_list)            ; namespace / class / struct / interface / record 的体
  (enum_member_declaration_list)
  (block)                       ; 方法体与语句块
  (accessor_list)               ; 属性的 get / set
  (switch_body)
  (initializer_expression)      ; 数组与对象初始化器
  (comment)                     ; 跨行注释（/* … */、/** … */）
] @fold
