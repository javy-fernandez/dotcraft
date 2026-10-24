; XML (.csproj, .props, .config…) in Rider's colours: the whole tag is purple, attribute
; names green and values brown.
(STag) @tag
(ETag) @tag
(EmptyElemTag) @tag
(XMLDecl) @tag
(PI) @tag
(doctypedecl) @tag
(Attribute (Name) @xml.attribute)
(Attribute (AttValue) @string)
(PseudoAtt (Name) @xml.attribute)
(PseudoAtt (PseudoAttValue) @string)
(EncName) @string
(VersionNum) @string
(Attribute "=" @tag)
(EntityRef) @entity
(CharRef) @entity
(CharData) @none
(CDSect) @string
(Comment) @comment
