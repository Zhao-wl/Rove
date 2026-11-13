
(Comment) @comment.block.xml
(CData) @string.unquoted.cdata.xml
[(EntityRef) (CharRef)] @constant.character.entity.xml
(STag (Name) @entity.name.tag.xml @entity.name.tag.localname.xml)
(ETag (Name) @entity.name.tag.xml @entity.name.tag.localname.xml)
(EmptyElemTag (Name) @entity.name.tag.xml @entity.name.tag.localname.xml)
(Attribute (Name) @entity.other.attribute-name.xml @entity.other.attribute-name.localname.xml)
((AttValue) @string.quoted.double.xml (#match? @string.quoted.double.xml "^\""))
((AttValue) @string.quoted.single.xml (#match? @string.quoted.single.xml "^'"))
[(STag) (ETag)] @meta.tag.xml
(EmptyElemTag) @meta.tag.no-content.xml
[(XMLDecl) (PI)] @meta.tag.preprocessor.xml
; XML 声明里的 xml / version / encoding / standalone 与它们的值（语法把这些写成匿名 token）。
(XMLDecl "xml" @entity.name.tag.xml @entity.name.tag.localname.xml)
(XMLDecl ["version" "encoding" "standalone"] @entity.other.attribute-name.xml @entity.other.attribute-name.localname.xml)
(XMLDecl [(VersionNum) (EncName) "yes" "no" "\"" "'"] @string.quoted.double.xml)
(PI (PITarget) @entity.name.tag.xml @entity.name.tag.localname.xml)
(doctypedecl) @meta.tag.sgml.doctype.xml
"DOCTYPE" @keyword.other.doctype.xml
"ENTITY" @keyword.other.entity.xml
["<?" "?>" "<!" "<" ">" "</" "/>"] @punctuation.definition.tag.xml
