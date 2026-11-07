; Shared JavaScript / TypeScript syntax. Queries are authored for CrabCraft.
(comment) @comment
(string) @string
(template_string) @string
(escape_sequence) @string.escape
(number) @number
(regex) @string
[(true) (false) (null) (undefined)] @constant
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function.call)
(property_identifier) @property
(class_declaration name: (_) @type.class)
["const" "let" "var" "function" "return" "if" "else" "for" "while" "do" "switch" "case" "break" "continue" "throw" "try" "catch" "finally" "class" "extends" "new" "import" "export" "from" "async" "await" "yield" "typeof" "instanceof" "delete" "void"] @keyword
["{" "}" "(" ")" "[" "]" ";" "," "."] @punctuation
["=" "+" "-" "*" "/" "==" "===" "!=" "!==" "=>" "!" "&&" "||" "??"] @operator
