use std::collections::HashMap;
use std::path::Path;

use rustdoc_types::{Crate, Id, Item, ItemEnum, Type};

/// A generated MDX file with its relative path and content.
#[derive(Debug, Clone)]
pub struct MdxFile {
    /// Relative path (e.g., `my_crate/utils.mdx`).
    pub path: String,
    /// MDX content.
    pub content: String,
}

/// Transform a rustdoc JSON `Crate` into a set of MDX files.
///
/// Generates one MDX file per module, with all items in that module
/// grouped by kind (structs, enums, functions, traits, etc.).
pub fn transform_to_mdx(krate: &Crate) -> Vec<MdxFile> {
    let mut files = Vec::new();
    let mut module_items: HashMap<String, Vec<(&Item, &str)>> = HashMap::new();

    for (id, item) in &krate.index {
        let module_path = resolve_module_path(krate, id);
        let kind_label = item_kind_label(&item.inner);
        if let Some(label) = kind_label {
            module_items
                .entry(module_path)
                .or_default()
                .push((item, label));
        }
    }

    let mut paths: Vec<_> = module_items.keys().cloned().collect();
    paths.sort();

    for module_path in paths {
        let items = &module_items[&module_path];
        let content = render_module(&module_path, items, krate);
        let file_path = if module_path.is_empty() {
            "index.mdx".to_string()
        } else {
            format!("{}.mdx", module_path.replace("::", "/"))
        };
        files.push(MdxFile {
            path: file_path,
            content,
        });
    }

    files
}

/// Write MDX files to the output directory.
pub fn write_mdx_files(files: &[MdxFile], output_dir: &Path) -> std::io::Result<()> {
    for file in files {
        let path = output_dir.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.content)?;
    }
    Ok(())
}

fn resolve_module_path(krate: &Crate, id: &Id) -> String {
    if let Some(summary) = krate.paths.get(id) {
        let path = &summary.path;
        if path.len() > 1 {
            path[..path.len() - 1].join("::")
        } else if !path.is_empty() {
            path[0].clone()
        } else {
            String::new()
        }
    } else {
        String::new()
    }
}

fn item_kind_label(inner: &ItemEnum) -> Option<&'static str> {
    match inner {
        ItemEnum::Module(_) => Some("Modules"),
        ItemEnum::Struct(_) => Some("Structs"),
        ItemEnum::Enum(_) => Some("Enums"),
        ItemEnum::Function(_) => Some("Functions"),
        ItemEnum::Trait(_) => Some("Traits"),
        ItemEnum::TypeAlias(_) => Some("Type Aliases"),
        ItemEnum::Constant { .. } => Some("Constants"),
        ItemEnum::Static(_) => Some("Statics"),
        ItemEnum::Macro(_) => Some("Macros"),
        _ => None,
    }
}

fn render_module(module_path: &str, items: &[(&Item, &str)], krate: &Crate) -> String {
    let mut out = String::new();

    let title = if module_path.is_empty() {
        krate
            .index
            .get(&krate.root)
            .and_then(|i| i.name.clone())
            .unwrap_or_else(|| "crate".to_string())
    } else {
        module_path.to_string()
    };

    let description = if module_path.is_empty() {
        krate
            .index
            .get(&krate.root)
            .and_then(|i| i.docs.as_ref())
            .map(|d| first_sentence(d))
            .unwrap_or_default()
    } else {
        items
            .iter()
            .find(|(item, kind)| {
                *kind == "Modules" && item.name.as_deref() == module_path.split("::").last()
            })
            .and_then(|(item, _)| item.docs.as_ref())
            .map(|d| first_sentence(d))
            .unwrap_or_default()
    };

    out.push_str(&format!(
        "---\ntitle: \"{title}\"\ndescription: \"{}\"\n---\n\n",
        description.replace('"', "\\\"")
    ));

    let mut by_kind: HashMap<&str, Vec<&Item>> = HashMap::new();
    for (item, kind) in items {
        by_kind.entry(kind).or_default().push(item);
    }

    let kind_order = [
        "Modules",
        "Structs",
        "Enums",
        "Traits",
        "Functions",
        "Type Aliases",
        "Constants",
        "Statics",
        "Macros",
    ];

    for kind in &kind_order {
        if let Some(kind_items) = by_kind.get(kind) {
            let mut sorted: Vec<_> = kind_items.iter().collect();
            sorted.sort_by_key(|item| &item.name);

            out.push_str(&format!("## {kind}\n\n"));

            for item in sorted {
                render_item(&mut out, item, krate);
            }
        }
    }

    out
}

fn render_item(out: &mut String, item: &Item, krate: &Crate) {
    let name = item.name.as_deref().unwrap_or("_");

    out.push_str(&format!("### `{name}`\n\n"));

    if let Some(sig) = render_signature(item, krate) {
        out.push_str("```rust\n");
        out.push_str(&sig);
        out.push('\n');
        out.push_str("```\n\n");
    }

    if let Some(ref docs) = item.docs {
        out.push_str(docs);
        out.push_str("\n\n");
    }
}

fn render_signature(item: &Item, krate: &Crate) -> Option<String> {
    let name = item.name.as_deref()?;
    match &item.inner {
        ItemEnum::Function(f) => {
            let mut sig = String::new();
            if f.header.is_const {
                sig.push_str("const ");
            }
            if f.header.is_async {
                sig.push_str("async ");
            }
            if f.header.is_unsafe {
                sig.push_str("unsafe ");
            }
            sig.push_str("fn ");
            sig.push_str(name);
            if !f.generics.params.is_empty() {
                sig.push('<');
                let params: Vec<_> = f.generics.params.iter().map(|p| p.name.clone()).collect();
                sig.push_str(&params.join(", "));
                sig.push('>');
            }
            sig.push('(');
            let params: Vec<_> = f
                .sig
                .inputs
                .iter()
                .map(|(pname, ty)| format!("{pname}: {}", render_type(ty, krate)))
                .collect();
            sig.push_str(&params.join(", "));
            sig.push(')');
            if let Some(ref output) = f.sig.output {
                sig.push_str(&format!(" -> {}", render_type(output, krate)));
            }
            Some(sig)
        }
        ItemEnum::Struct(s) => {
            let mut sig = String::from("pub struct ");
            sig.push_str(name);
            if !s.generics.params.is_empty() {
                sig.push('<');
                let params: Vec<_> = s.generics.params.iter().map(|p| p.name.clone()).collect();
                sig.push_str(&params.join(", "));
                sig.push('>');
            }
            match &s.kind {
                rustdoc_types::StructKind::Unit => sig.push(';'),
                rustdoc_types::StructKind::Tuple(_) => sig.push_str("(...)"),
                rustdoc_types::StructKind::Plain { fields, .. } => {
                    sig.push_str(" { ");
                    let field_names: Vec<_> = fields
                        .iter()
                        .filter_map(|fid| krate.index.get(fid))
                        .filter_map(|f| f.name.as_deref())
                        .collect();
                    sig.push_str(&field_names.join(", "));
                    sig.push_str(" }");
                }
            }
            Some(sig)
        }
        ItemEnum::Enum(e) => {
            let mut sig = String::from("pub enum ");
            sig.push_str(name);
            if !e.generics.params.is_empty() {
                sig.push('<');
                let params: Vec<_> = e.generics.params.iter().map(|p| p.name.clone()).collect();
                sig.push_str(&params.join(", "));
                sig.push('>');
            }
            sig.push_str(" { ");
            let variant_names: Vec<_> = e
                .variants
                .iter()
                .filter_map(|vid| krate.index.get(vid))
                .filter_map(|v| v.name.as_deref())
                .collect();
            sig.push_str(&variant_names.join(", "));
            sig.push_str(" }");
            Some(sig)
        }
        ItemEnum::Trait(t) => {
            let mut sig = String::from("pub trait ");
            sig.push_str(name);
            if !t.generics.params.is_empty() {
                sig.push('<');
                let params: Vec<_> = t.generics.params.iter().map(|p| p.name.clone()).collect();
                sig.push_str(&params.join(", "));
                sig.push('>');
            }
            Some(sig)
        }
        ItemEnum::TypeAlias(ta) => Some(format!(
            "pub type {name} = {}",
            render_type(&ta.type_, krate)
        )),
        ItemEnum::Constant { type_, const_: c } => Some(format!(
            "pub const {name}: {} = {}",
            render_type(type_, krate),
            c.value.as_deref().unwrap_or("...")
        )),
        ItemEnum::Macro(_) => Some(format!("macro_rules! {name} {{ ... }}")),
        _ => None,
    }
}

fn render_type(ty: &Type, krate: &Crate) -> String {
    match ty {
        Type::ResolvedPath(p) => {
            let mut s = p.path.clone();
            if let Some(ref args) = p.args {
                if let rustdoc_types::GenericArgs::AngleBracketed { args, .. } = args.as_ref() {
                    if !args.is_empty() {
                        s.push('<');
                        let rendered: Vec<_> = args
                            .iter()
                            .map(|a| match a {
                                rustdoc_types::GenericArg::Type(t) => render_type(t, krate),
                                rustdoc_types::GenericArg::Lifetime(l) => l.clone(),
                                rustdoc_types::GenericArg::Const(c) => {
                                    c.value.clone().unwrap_or_else(|| c.expr.clone())
                                }
                                rustdoc_types::GenericArg::Infer => "_".to_string(),
                            })
                            .collect();
                        s.push_str(&rendered.join(", "));
                        s.push('>');
                    }
                }
            }
            s
        }
        Type::Generic(name) => name.clone(),
        Type::Primitive(name) => name.clone(),
        Type::BorrowedRef {
            lifetime,
            is_mutable,
            type_,
        } => {
            let mut s = String::from("&");
            if let Some(lt) = lifetime {
                s.push_str(lt);
                s.push(' ');
            }
            if *is_mutable {
                s.push_str("mut ");
            }
            s.push_str(&render_type(type_, krate));
            s
        }
        Type::Tuple(types) => {
            let inner: Vec<_> = types.iter().map(|t| render_type(t, krate)).collect();
            format!("({})", inner.join(", "))
        }
        Type::Slice(ty) => format!("[{}]", render_type(ty, krate)),
        Type::Array { type_, len } => format!("[{}; {}]", render_type(type_, krate), len),
        Type::RawPointer { is_mutable, type_ } => {
            if *is_mutable {
                format!("*mut {}", render_type(type_, krate))
            } else {
                format!("*const {}", render_type(type_, krate))
            }
        }
        Type::ImplTrait(bounds) => {
            let rendered: Vec<_> = bounds
                .iter()
                .filter_map(|b| match b {
                    rustdoc_types::GenericBound::TraitBound { trait_, .. } => {
                        Some(trait_.path.clone())
                    }
                    _ => None,
                })
                .collect();
            format!("impl {}", rendered.join(" + "))
        }
        Type::QualifiedPath {
            name,
            self_type,
            trait_,
            ..
        } => {
            let self_str = render_type(self_type, krate);
            if let Some(t) = trait_ {
                format!("<{self_str} as {}>::{name}", t.path)
            } else {
                format!("{self_str}::{name}")
            }
        }
        Type::DynTrait(dt) => {
            let traits: Vec<_> = dt.traits.iter().map(|pb| pb.trait_.path.clone()).collect();
            format!("dyn {}", traits.join(" + "))
        }
        Type::FunctionPointer(fp) => {
            let params: Vec<_> = fp
                .sig
                .inputs
                .iter()
                .map(|(_, t)| render_type(t, krate))
                .collect();
            let ret = fp
                .sig
                .output
                .as_ref()
                .map(|t| format!(" -> {}", render_type(t, krate)))
                .unwrap_or_default();
            format!("fn({}){ret}", params.join(", "))
        }
        Type::Pat { type_, .. } => render_type(type_, krate),
        Type::Infer => "_".to_string(),
    }
}

fn first_sentence(docs: &str) -> String {
    docs.split('\n')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustdoc_types::*;

    fn empty_crate() -> Crate {
        Crate {
            root: Id(0),
            crate_version: Some("0.1.0".to_string()),
            includes_private: false,
            index: HashMap::new(),
            paths: HashMap::new(),
            external_crates: HashMap::new(),
            format_version: 38,
        }
    }

    fn make_function(name: &str, params: Vec<(&str, Type)>, output: Option<Type>) -> Item {
        Item {
            id: Id(1),
            crate_id: 0,
            name: Some(name.to_string()),
            span: None,
            visibility: Visibility::Public,
            docs: Some(format!("Documentation for {name}.")),
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Function(Function {
                sig: FunctionSignature {
                    inputs: params
                        .into_iter()
                        .map(|(n, t)| (n.to_string(), t))
                        .collect(),
                    output,
                    is_c_variadic: false,
                },
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![],
                },
                header: FunctionHeader {
                    is_const: false,
                    is_unsafe: false,
                    is_async: false,
                    abi: Abi::Rust,
                },
                has_body: true,
            }),
        }
    }

    fn make_struct(name: &str) -> Item {
        Item {
            id: Id(2),
            crate_id: 0,
            name: Some(name.to_string()),
            span: None,
            visibility: Visibility::Public,
            docs: Some(format!("A {name} struct.")),
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Struct(Struct {
                kind: StructKind::Unit,
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![],
                },
                impls: vec![],
            }),
        }
    }

    fn make_enum(name: &str) -> Item {
        Item {
            id: Id(3),
            crate_id: 0,
            name: Some(name.to_string()),
            span: None,
            visibility: Visibility::Public,
            docs: Some(format!("The {name} enum.")),
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Enum(Enum {
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![],
                },
                variants: vec![],
                has_stripped_variants: false,
                impls: vec![],
            }),
        }
    }

    fn make_trait(name: &str) -> Item {
        Item {
            id: Id(4),
            crate_id: 0,
            name: Some(name.to_string()),
            span: None,
            visibility: Visibility::Public,
            docs: Some(format!("The {name} trait.")),
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Trait(Trait {
                is_auto: false,
                is_unsafe: false,
                is_dyn_compatible: true,
                items: vec![],
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![],
                },
                bounds: vec![],
                implementations: vec![],
            }),
        }
    }

    #[test]
    fn first_sentence_basic() {
        assert_eq!(first_sentence("Hello world."), "Hello world");
        assert_eq!(first_sentence("First line.\nSecond line."), "First line");
        assert_eq!(first_sentence(""), "");
    }

    #[test]
    fn render_type_primitives() {
        let krate = empty_crate();
        assert_eq!(render_type(&Type::Primitive("u32".into()), &krate), "u32");
        assert_eq!(render_type(&Type::Primitive("bool".into()), &krate), "bool");
    }

    #[test]
    fn render_type_generic() {
        let krate = empty_crate();
        assert_eq!(render_type(&Type::Generic("T".into()), &krate), "T");
    }

    #[test]
    fn render_type_reference() {
        let krate = empty_crate();
        let ty = Type::BorrowedRef {
            lifetime: Some("'a".into()),
            is_mutable: false,
            type_: Box::new(Type::Primitive("str".into())),
        };
        assert_eq!(render_type(&ty, &krate), "&'a str");
    }

    #[test]
    fn render_type_mut_reference() {
        let krate = empty_crate();
        let ty = Type::BorrowedRef {
            lifetime: None,
            is_mutable: true,
            type_: Box::new(Type::Primitive("u8".into())),
        };
        assert_eq!(render_type(&ty, &krate), "&mut u8");
    }

    #[test]
    fn render_type_tuple() {
        let krate = empty_crate();
        let ty = Type::Tuple(vec![
            Type::Primitive("u32".into()),
            Type::Primitive("String".into()),
        ]);
        assert_eq!(render_type(&ty, &krate), "(u32, String)");
    }

    #[test]
    fn render_type_slice() {
        let krate = empty_crate();
        assert_eq!(
            render_type(&Type::Slice(Box::new(Type::Primitive("u8".into()))), &krate),
            "[u8]"
        );
    }

    #[test]
    fn render_type_array() {
        let krate = empty_crate();
        let ty = Type::Array {
            type_: Box::new(Type::Primitive("u8".into())),
            len: "32".into(),
        };
        assert_eq!(render_type(&ty, &krate), "[u8; 32]");
    }

    #[test]
    fn render_type_raw_pointer() {
        let krate = empty_crate();
        let ty = Type::RawPointer {
            is_mutable: true,
            type_: Box::new(Type::Primitive("u8".into())),
        };
        assert_eq!(render_type(&ty, &krate), "*mut u8");
    }

    #[test]
    fn render_function_signature() {
        let krate = empty_crate();
        let item = make_function(
            "add",
            vec![
                ("a", Type::Primitive("u32".into())),
                ("b", Type::Primitive("u32".into())),
            ],
            Some(Type::Primitive("u32".into())),
        );
        assert_eq!(
            render_signature(&item, &krate).unwrap(),
            "fn add(a: u32, b: u32) -> u32"
        );
    }

    #[test]
    fn render_function_no_return() {
        let krate = empty_crate();
        let item = make_function("do_thing", vec![], None);
        assert_eq!(render_signature(&item, &krate).unwrap(), "fn do_thing()");
    }

    #[test]
    fn render_struct_signature() {
        let krate = empty_crate();
        assert_eq!(
            render_signature(&make_struct("MyStruct"), &krate).unwrap(),
            "pub struct MyStruct;"
        );
    }

    #[test]
    fn render_enum_signature() {
        let krate = empty_crate();
        assert_eq!(
            render_signature(&make_enum("Color"), &krate).unwrap(),
            "pub enum Color {  }"
        );
    }

    #[test]
    fn render_trait_signature() {
        let krate = empty_crate();
        assert_eq!(
            render_signature(&make_trait("Drawable"), &krate).unwrap(),
            "pub trait Drawable"
        );
    }

    #[test]
    fn item_kind_labels() {
        assert_eq!(
            item_kind_label(&ItemEnum::Module(Module {
                is_crate: false,
                items: vec![],
                is_stripped: false
            })),
            Some("Modules")
        );
        assert_eq!(
            item_kind_label(&ItemEnum::Struct(Struct {
                kind: StructKind::Unit,
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![]
                },
                impls: vec![]
            })),
            Some("Structs")
        );
    }

    #[test]
    fn transform_empty_crate() {
        assert!(transform_to_mdx(&empty_crate()).is_empty());
    }

    #[test]
    fn transform_crate_with_function() {
        let mut krate = empty_crate();
        let func = make_function("hello", vec![], None);
        let id = Id(1);
        krate.index.insert(id.clone(), func);
        krate.paths.insert(
            id,
            ItemSummary {
                crate_id: 0,
                path: vec!["my_crate".into(), "hello".into()],
                kind: ItemKind::Function,
            },
        );

        let files = transform_to_mdx(&krate);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "my_crate.mdx");
        assert!(files[0].content.contains("### `hello`"));
        assert!(files[0].content.contains("fn hello()"));
        assert!(files[0].content.contains("Documentation for hello."));
    }

    #[test]
    fn transform_crate_with_multiple_kinds() {
        let mut krate = empty_crate();
        let func = make_function("do_thing", vec![], None);
        krate.index.insert(Id(1), func);
        krate.paths.insert(
            Id(1),
            ItemSummary {
                crate_id: 0,
                path: vec!["mc".into(), "do_thing".into()],
                kind: ItemKind::Function,
            },
        );

        let st = make_struct("Widget");
        krate.index.insert(Id(2), st);
        krate.paths.insert(
            Id(2),
            ItemSummary {
                crate_id: 0,
                path: vec!["mc".into(), "Widget".into()],
                kind: ItemKind::Struct,
            },
        );

        let files = transform_to_mdx(&krate);
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        assert!(content.find("## Structs").unwrap() < content.find("## Functions").unwrap());
    }

    #[test]
    fn frontmatter_escapes_quotes() {
        // Put a module with quoted docs as the root so it becomes the frontmatter description.
        let mut krate = empty_crate();
        let root_module = Item {
            id: Id(0),
            crate_id: 0,
            name: Some("mylib".into()),
            span: None,
            visibility: Visibility::Public,
            docs: Some("A \"quoted\" crate.".into()),
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Module(Module {
                is_crate: true,
                items: vec![Id(1)],
                is_stripped: false,
            }),
        };
        krate.root = Id(0);
        krate.index.insert(Id(0), root_module);

        // Add a function so the module generates a file.
        let func = make_function("f", vec![], None);
        krate.index.insert(Id(1), func);
        krate.paths.insert(
            Id(1),
            ItemSummary {
                crate_id: 0,
                path: vec!["f".into()],
                kind: ItemKind::Function,
            },
        );

        let files = transform_to_mdx(&krate);
        // The root module's description in frontmatter should have escaped quotes.
        let index = files.iter().find(|f| f.path == "index.mdx").unwrap();
        assert!(
            index.content.contains("\\\"quoted\\\""),
            "content: {}",
            index.content
        );
    }

    #[test]
    fn render_type_resolved_path_with_args() {
        let krate = empty_crate();
        let ty = Type::ResolvedPath(rustdoc_types::Path {
            path: "Option".into(),
            id: Id(99),
            args: Some(Box::new(rustdoc_types::GenericArgs::AngleBracketed {
                args: vec![rustdoc_types::GenericArg::Type(Type::Primitive(
                    "u32".into(),
                ))],
                constraints: vec![],
            })),
        });
        assert_eq!(render_type(&ty, &krate), "Option<u32>");
    }

    #[test]
    fn render_type_impl_trait() {
        let krate = empty_crate();
        let ty = Type::ImplTrait(vec![rustdoc_types::GenericBound::TraitBound {
            trait_: rustdoc_types::Path {
                path: "Display".into(),
                id: Id(99),
                args: None,
            },
            generic_params: vec![],
            modifier: rustdoc_types::TraitBoundModifier::None,
        }]);
        assert_eq!(render_type(&ty, &krate), "impl Display");
    }

    #[test]
    fn render_type_dyn_trait() {
        let krate = empty_crate();
        let ty = Type::DynTrait(rustdoc_types::DynTrait {
            traits: vec![rustdoc_types::PolyTrait {
                trait_: rustdoc_types::Path {
                    path: "Error".into(),
                    id: Id(99),
                    args: None,
                },
                generic_params: vec![],
            }],
            lifetime: None,
        });
        assert_eq!(render_type(&ty, &krate), "dyn Error");
    }

    #[test]
    fn render_type_function_pointer() {
        let krate = empty_crate();
        let ty = Type::FunctionPointer(Box::new(rustdoc_types::FunctionPointer {
            sig: FunctionSignature {
                inputs: vec![("x".into(), Type::Primitive("u32".into()))],
                output: Some(Type::Primitive("bool".into())),
                is_c_variadic: false,
            },
            generic_params: vec![],
            header: FunctionHeader {
                is_const: false,
                is_unsafe: false,
                is_async: false,
                abi: Abi::Rust,
            },
        }));
        assert_eq!(render_type(&ty, &krate), "fn(u32) -> bool");
    }

    #[test]
    fn render_type_const_pointer() {
        let krate = empty_crate();
        let ty = Type::RawPointer {
            is_mutable: false,
            type_: Box::new(Type::Primitive("u8".into())),
        };
        assert_eq!(render_type(&ty, &krate), "*const u8");
    }

    #[test]
    fn render_type_infer() {
        let krate = empty_crate();
        assert_eq!(render_type(&Type::Infer, &krate), "_");
    }

    #[test]
    fn render_type_qualified_path() {
        let krate = empty_crate();
        let ty = Type::QualifiedPath {
            name: "Item".into(),
            args: Box::new(rustdoc_types::GenericArgs::AngleBracketed {
                args: vec![],
                constraints: vec![],
            }),
            self_type: Box::new(Type::Generic("T".into())),
            trait_: Some(rustdoc_types::Path {
                path: "Iterator".into(),
                id: Id(99),
                args: None,
            }),
        };
        assert_eq!(render_type(&ty, &krate), "<T as Iterator>::Item");
    }

    #[test]
    fn item_kind_label_all_variants() {
        // Test the remaining untested variants
        assert_eq!(
            item_kind_label(&ItemEnum::Enum(Enum {
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![]
                },
                variants: vec![],
                has_stripped_variants: false,
                impls: vec![],
            })),
            Some("Enums")
        );
        assert_eq!(
            item_kind_label(&ItemEnum::Trait(Trait {
                is_auto: false,
                is_unsafe: false,
                is_dyn_compatible: true,
                items: vec![],
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![]
                },
                bounds: vec![],
                implementations: vec![],
            })),
            Some("Traits")
        );
        assert_eq!(item_kind_label(&ItemEnum::Macro("".into())), Some("Macros"));
        assert_eq!(
            item_kind_label(&ItemEnum::Static(rustdoc_types::Static {
                type_: Type::Primitive("u32".into()),
                is_mutable: false,
                is_unsafe: false,
                expr: String::new(),
            })),
            Some("Statics")
        );
        // Impl blocks should be skipped
        assert_eq!(
            item_kind_label(&ItemEnum::Impl(rustdoc_types::Impl {
                is_unsafe: false,
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![]
                },
                provided_trait_methods: vec![],
                trait_: None,
                for_: Type::Primitive("u32".into()),
                items: vec![],
                is_negative: false,
                is_synthetic: false,
                blanket_impl: None,
            })),
            None
        );
    }

    #[test]
    fn render_constant_signature() {
        let krate = empty_crate();
        let item = Item {
            id: Id(5),
            crate_id: 0,
            name: Some("MAX_SIZE".into()),
            span: None,
            visibility: Visibility::Public,
            docs: None,
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Constant {
                type_: Type::Primitive("usize".into()),
                const_: rustdoc_types::Constant {
                    expr: "1024".into(),
                    value: Some("1024".into()),
                    is_literal: true,
                },
            },
        };
        assert_eq!(
            render_signature(&item, &krate).unwrap(),
            "pub const MAX_SIZE: usize = 1024"
        );
    }

    #[test]
    fn render_type_alias_signature() {
        let krate = empty_crate();
        let item = Item {
            id: Id(6),
            crate_id: 0,
            name: Some("Result".into()),
            span: None,
            visibility: Visibility::Public,
            docs: None,
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::TypeAlias(rustdoc_types::TypeAlias {
                type_: Type::Primitive("u32".into()),
                generics: Generics {
                    params: vec![],
                    where_predicates: vec![],
                },
            }),
        };
        assert_eq!(
            render_signature(&item, &krate).unwrap(),
            "pub type Result = u32"
        );
    }

    #[test]
    fn render_macro_signature() {
        let krate = empty_crate();
        let item = Item {
            id: Id(7),
            crate_id: 0,
            name: Some("my_macro".into()),
            span: None,
            visibility: Visibility::Public,
            docs: None,
            links: HashMap::new(),
            attrs: vec![],
            deprecation: None,
            inner: ItemEnum::Macro("macro body".into()),
        };
        assert_eq!(
            render_signature(&item, &krate).unwrap(),
            "macro_rules! my_macro { ... }"
        );
    }

    #[test]
    fn render_item_without_docs() {
        let krate = empty_crate();
        let mut item = make_struct("NoDocs");
        item.docs = None;
        let mut out = String::new();
        render_item(&mut out, &item, &krate);
        assert!(out.contains("### `NoDocs`"));
        assert!(out.contains("pub struct NoDocs;"));
        // Should not have trailing doc content
        assert!(!out.contains("A NoDocs struct."));
    }

    #[test]
    fn write_mdx_files_creates_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let files = vec![MdxFile {
            path: "nested/module.mdx".into(),
            content: "# Test\n".into(),
        }];
        write_mdx_files(&files, tmp.path()).unwrap();
        assert!(tmp.path().join("nested/module.mdx").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("nested/module.mdx")).unwrap(),
            "# Test\n"
        );
    }
}
