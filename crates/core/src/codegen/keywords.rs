//! Names a variable option must not take: keywords of every supported
//! language plus the identifiers the generated snippets define themselves.

/// Compared case-insensitively (PHP keywords are case-insensitive).
#[rustfmt::skip]
const RESERVED: &[&str] = &[
    // JavaScript
    "arguments", "async", "await", "const", "debugger", "delete", "eval", "export", "function",
    "let", "null", "of", "typeof", "undefined", "var", "with", "yield", "NaN", "Infinity",
    // Python
    "and", "as", "assert", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not",
    "or", "pass", "raise", "return", "try", "while", "None", "True", "False", "print",
    // Java
    "abstract", "boolean", "byte", "case", "catch", "char", "default", "do", "double", "enum",
    "extends", "final", "float", "goto", "implements", "instanceof", "int", "interface", "long",
    "native", "new", "package", "private", "protected", "public", "record", "short", "static",
    "strictfp", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "void",
    "volatile", "true", "false",
    // C#
    "base", "bool", "checked", "decimal", "delegate", "event", "explicit", "extern", "fixed",
    "foreach", "implicit", "internal", "lock", "namespace", "object", "operator", "out",
    "override", "params", "readonly", "ref", "sbyte", "sealed", "sizeof", "stackalloc", "string",
    "struct", "uint", "ulong", "unchecked", "unsafe", "ushort", "using", "virtual",
    // Go (keywords and predeclared identifiers the snippets rely on)
    "chan", "defer", "fallthrough", "func", "go", "map", "range", "select", "type", "nil", "iota",
    "error", "any", "len", "make", "append", "uint8", "int32", "int64", "float64",
    // PHP
    "array", "callable", "clone", "declare", "die", "echo", "elseif", "empty", "enddeclare",
    "endfor", "endforeach", "endif", "endswitch", "endwhile", "exit", "fn", "include",
    "include_once", "insteadof", "isset", "list", "match", "require", "require_once", "trait",
    "unset", "use", "xor",
    // Ruby
    "alias", "begin", "end", "defined", "ensure", "elsif", "module", "next", "redo", "rescue",
    "retry", "self", "then", "undef", "unless", "until", "when", "puts",
    // Rust
    "crate", "dyn", "impl", "loop", "mod", "move", "mut", "pub", "where", "box", "become",
    "macro", "priv", "unsized", "main", "Vec", "Some", "Ok", "Err", "Result",
    // Locals and types the generated snippets define or import
    "pipeline", "filter", "sort", "projection", "results", "cursor", "options", "doc", "bson",
    "mongodb", "MongoClient", "ObjectId", "Long", "Int32", "Double", "Decimal128", "Binary",
    "Timestamp", "MinKey", "MaxKey", "Code", "UUID", "DBRef", "BSONRegExp", "BSONSymbol", "Regex",
    "Int64", "DatetimeMS", "datetime", "Document", "Date", "ISODate", "load", "quit", "sleep",
    "printjson", "console", "exports", "process", "Buffer",
    // Classes and packages the Java, C#, Go, PHP and Ruby snippets rely on
    "Main", "String", "System", "Arrays", "List", "ArrayList", "MongoClients", "MongoDatabase",
    "MongoCollection", "BsonRegularExpression", "BsonTimestamp", "BsonUndefined", "BsonDbPointer",
    "CodeWithScope", "Symbol", "BsonDocument", "BsonArray", "BsonDateTime", "BsonBinaryData",
    "BsonNull", "BsonMinKey", "BsonMaxKey", "BsonJavaScript", "BsonSymbolTable", "Console",
    "MongoDB", "err", "opts", "item", "DateTime", "gen", "Bson", "Self",
    // Built-ins and helpers the generated values call: the shell's `NumberLong` family, Node's
    // `Map` for ordered documents, Python's `bytes` and `float`, the `args` parameter of the
    // Java and C# entry points, C# subtype and scope classes, PHP's `hex2bin`.
    "Map", "NumberLong", "NumberDecimal", "BinData", "bytes", "float", "args",
    "BsonBinarySubType", "BsonJavaScriptWithScope", "hex2bin",
];

pub(crate) fn is_reserved_name(name: &str) -> bool {
    RESERVED.iter().any(|k| k.eq_ignore_ascii_case(name))
}
