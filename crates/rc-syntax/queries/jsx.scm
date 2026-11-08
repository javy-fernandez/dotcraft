; JSX markup is shared by the JavaScript and TSX grammars, but not plain TypeScript.
(jsx_opening_element name: (_) @tag)
(jsx_closing_element name: (_) @tag)
(jsx_self_closing_element name: (_) @tag)

; Member components retain tag styling on their property, overriding JS property roles.
(jsx_opening_element name: (member_expression property: (property_identifier) @tag))
(jsx_closing_element name: (member_expression property: (property_identifier) @tag))
(jsx_self_closing_element name: (member_expression property: (property_identifier) @tag))
(jsx_attribute (property_identifier) @xml.attribute)
(jsx_opening_element ["<" ">"] @punctuation)
(jsx_closing_element ["</" ">"] @punctuation)
(jsx_self_closing_element ["<" "/>"] @punctuation)
