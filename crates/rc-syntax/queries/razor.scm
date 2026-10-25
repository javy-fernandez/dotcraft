; Razor (.razor, .cshtml) on top of csharp.scm: directives and transitions as keywords (Rider),
; Razor and HTML comments, the doctype as a tag. HTML elements are coloured by a lexer
; (`highlight::html_spans`): the grammar leaves them to an HTML injection.
[
  (razor_comment)
  (html_comment)
] @comment

(html_doctype) @tag

[
  "at_page"
  "at_using"
  "at_model"
  "at_rendermode"
  "at_inject"
  "at_implements"
  "at_layout"
  "at_inherits"
  "at_attribute"
  "at_typeparam"
  "at_namespace"
  "at_preservewhitespace"
  "at_block"
  "at_addtaghelper"
  "at_removetaghelper"
  "at_taghelperprefix"
  "at_lock"
  "at_section"
  "at_if"
  "at_switch"
  "at_for"
  "at_foreach"
  "at_while"
  "at_do"
  "at_try"
  "at_implicit"
  "at_explicit"
  "at_at_escape"
  "at_colon_transition"
] @keyword

(razor_rendermode) @property
(razor_attribute_name) @attribute
