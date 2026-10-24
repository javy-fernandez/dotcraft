; C# highlighting for DotCraft, tuned to Rider's colour roles. When captures overlap, the
; smaller node wins, and for the same node the later pattern wins.

(identifier) @variable

; ---- types
(predefined_type) @keyword
(implicit_type) @keyword
(class_declaration name: (identifier) @type.class)
(record_declaration name: (identifier) @type.class)
(interface_declaration name: (identifier) @type.interface)
(struct_declaration name: (identifier) @type.struct)
(enum_declaration name: (identifier) @type.enum)
(delegate_declaration name: (identifier) @type.delegate)
(type_parameter name: (identifier) @type.parameter)
(type_parameter_constraints_clause (identifier) @type.parameter)
(constructor_declaration name: (identifier) @type.class)
(destructor_declaration name: (identifier) @type.class)
(base_list (identifier) @type)
(base_list (generic_name (identifier) @type))
(primary_constructor_base_type type: (identifier) @type)
(_ type: (identifier) @type)
(_ type: (generic_name (identifier) @type))
(_ type: (qualified_name name: (identifier) @type))
(_ type: (nullable_type type: (identifier) @type))
(_ type: (array_type type: (identifier) @type))
(type_argument_list (identifier) @type)
(generic_name (identifier) @type)
(object_creation_expression type: (identifier) @type.class)
(typeof_expression type: (identifier) @type)
(as_expression right: (identifier) @type)
(is_expression right: (identifier) @type)
(cast_expression type: (identifier) @type)
(declaration_pattern type: (identifier) @type)
(type_pattern type: (identifier) @type)
(catch_declaration type: (identifier) @type)
(attribute name: (identifier) @attribute)
(attribute name: (qualified_name name: (identifier) @attribute))

; ---- namespaces
(namespace_declaration name: (identifier) @namespace)
(namespace_declaration name: (qualified_name) @namespace)
(file_scoped_namespace_declaration name: (identifier) @namespace)
(file_scoped_namespace_declaration name: (qualified_name) @namespace)
(using_directive (identifier) @namespace)
(using_directive (qualified_name) @namespace)
(qualified_name (identifier) @namespace)

; ---- members
(method_declaration name: (identifier) @function)
(local_function_statement name: (identifier) @function)
(operator_declaration "operator" @keyword)
(invocation_expression function: (identifier) @function.call)
(invocation_expression function: (member_access_expression name: (identifier) @function.call))
(invocation_expression function: (member_access_expression name: (generic_name (identifier) @function.call)))
(invocation_expression function: (generic_name (identifier) @function.call))
(invocation_expression function: (conditional_access_expression (member_binding_expression name: (identifier) @function.call)))
(property_declaration name: (identifier) @property)
(event_declaration name: (identifier) @event)
(event_field_declaration (variable_declaration (variable_declarator name: (identifier) @event)))
(field_declaration (variable_declaration (variable_declarator name: (identifier) @field)))
(enum_member_declaration name: (identifier) @constant.enum)
(parameter name: (identifier) @variable.parameter)
(variable_declarator name: (identifier) @variable.local)

; ---- literals
[(integer_literal) (real_literal)] @number
[(string_literal) (verbatim_string_literal) (raw_string_literal) (character_literal) (interpolated_string_expression)] @string
[(interpolation_start) (interpolation_quote)] @string
(interpolation) @none
(interpolation_brace) @string.brace
(escape_sequence) @string.escape
[(boolean_literal) (null_literal)] @keyword

; ---- comments
(comment) @comment
((comment) @comment.doc (#match? @comment.doc "^///"))

; ---- preprocessor
[(preproc_region) (preproc_endregion) (preproc_if) (preproc_elif) (preproc_else) (preproc_define) (preproc_undef) (preproc_pragma) (preproc_nullable) (preproc_warning) (preproc_error) (preproc_line)] @preproc
(preproc_arg) @preproc.arg

; ---- punctuation & operators
[";" "." "," ":"] @punctuation
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["=" "==" "!=" "<" "<=" ">" ">=" "+" "-" "*" "/" "%" "!" "&&" "||" "&" "|" "^" "~" "?" "??" "??=" "=>" "++" "--" "+=" "-=" "*=" "/=" "%=" "&=" "|=" "^=" "<<" ">>" "<<=" ">>=" ".."] @operator

; ---- keywords
(modifier) @keyword
["abstract" "add" "alias" "as" "ascending" "async" "await" "base" "break" "by" "case" "catch" "checked" "class" "const" "continue" "default" "delegate" "descending" "do" "else" "enum" "equals" "event" "explicit" "extern" "file" "finally" "fixed" "for" "foreach" "from" "get" "global" "goto" "group" "if" "implicit" "in" "init" "interface" "internal" "into" "is" "join" "let" "lock" "managed" "namespace" "new" "not" "notnull" "on" "operator" "or" "and" "orderby" "out" "override" "params" "partial" "private" "protected" "public" "readonly" "record" "ref" "remove" "required" "return" "scoped" "sealed" "select" "set" "sizeof" "stackalloc" "static" "struct" "switch" "this" "throw" "try" "typeof" "unchecked" "unmanaged" "unsafe" "using" "var" "virtual" "volatile" "when" "where" "while" "with" "yield"] @keyword
