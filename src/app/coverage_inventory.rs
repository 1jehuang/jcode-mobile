//! Syntax-aware change detector for the interaction coverage ledger.
//! This inventories wiring, not semantic reachability or exhaustive behavior.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use syn::parse::Parser as _;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Item, Meta, Token, Type, punctuated::Punctuated};

const LEDGER: &str = "docs/MOBILE_INTERACTION_COVERAGE.json";

#[derive(Default)]
struct Inventory {
    functions: BTreeMap<String, Value>,
    tests: BTreeSet<String>,
    visited: BTreeSet<(PathBuf, bool)>,
}

fn has_test_attr(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test"))
}

// Evaluate only the `test` predicate, leaving target/feature predicates unknown.
// This includes both iOS and desktop branches but excludes cfg(all(test, ...)).
fn without_test(meta: &Meta) -> Option<bool> {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Some(false),
        Meta::List(list) => {
            let args = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            let values: Vec<_> = args.iter().map(without_test).collect();
            if list.path.is_ident("not") && values.len() == 1 {
                values[0].map(|v| !v)
            } else if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn test_only(attrs: &[Attribute]) -> bool {
    has_test_attr(attrs)
        || attrs.iter().any(|a| {
            a.path().is_ident("cfg")
                && a.parse_args::<Meta>()
                    .ok()
                    .is_some_and(|m| without_test(&m) == Some(false))
        })
}

fn path_label(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn type_label(ty: &Type) -> String {
    match ty {
        Type::Path(p) => path_label(&p.path),
        Type::Reference(r) => format!("&{}", type_label(&r.elem)),
        _ => "anonymous-type".into(),
    }
}

fn element_type(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "AnyElement" | "Div" | "Stateful" | "Element" | "InteractiveElement" | "TextInput"
            )
        }),
        Type::ImplTrait(opaque) => opaque.bounds.iter().any(|bound| match bound {
            syn::TypeParamBound::Trait(bound) => bound
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "IntoElement"),
            _ => false,
        }),
        _ => false,
    }
}

// Keep literal IDs and format/tuple families, rather than one row per runtime ID.
// Unknown expression kinds remain explicit dynamic families, never silently omitted.
fn family(expr: &Expr) -> String {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Str(s) => format!("literal:{}", s.value()),
            _ => "dynamic:literal".into(),
        },
        Expr::Path(p) => format!("dynamic:{}", path_label(&p.path)),
        Expr::Tuple(t) => format!("tuple:{}", t.elems.first().map(family).unwrap_or_default()),
        Expr::Macro(m) => {
            let args = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(m.mac.tokens.clone());
            let first = args
                .ok()
                .and_then(|args| args.first().map(family))
                .unwrap_or_else(|| "dynamic:tokens".into());
            format!("macro:{}:{first}", path_label(&m.mac.path))
        }
        Expr::Closure(c) => family(&c.body),
        Expr::MethodCall(c) => family(&c.receiver),
        Expr::Call(c) => c
            .args
            .first()
            .map(family)
            .unwrap_or_else(|| "dynamic:call".into()),
        Expr::Paren(p) => family(&p.expr),
        Expr::Reference(r) => family(&r.expr),
        Expr::Block(_) => "dynamic:block".into(),
        Expr::If(_) => "dynamic:if".into(),
        Expr::Match(_) => "dynamic:match".into(),
        _ => "dynamic:expression".into(),
    }
}

#[derive(Default)]
struct Wiring {
    events: BTreeMap<String, usize>,
    controls: BTreeMap<String, usize>,
}

impl<'ast> Visit<'ast> for Wiring {
    fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {
        // A nested function is not executed just because its parent renders.
        // Production nested functions are inventoried separately by Inventory.
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if name.starts_with("on_") || name.starts_with("capture_") {
            *self.events.entry(name.clone()).or_default() += 1;
        }
        if matches!(name.as_str(), "id" | "debug_selector") {
            let value = call
                .args
                .first()
                .map(family)
                .unwrap_or_else(|| "dynamic:missing".into());
            *self.controls.entry(format!("{name}:{value}")).or_default() += 1;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let Expr::Path(path) = &*call.func {
            let name = path.path.segments.last().unwrap().ident.to_string();
            if name.starts_with("on_") || name.starts_with("capture_") {
                *self.events.entry(name.clone()).or_default() += 1;
            }
            if matches!(
                name.as_str(),
                "pill" | "outline_button" | "round_button" | "row"
            ) && !call.args.is_empty()
            {
                let value = call
                    .args
                    .first()
                    .map(family)
                    .unwrap_or_else(|| "dynamic:missing".into());
                *self.controls.entry(format!("{name}:{value}")).or_default() += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Test-only blocks inside otherwise production functions do not count.
        let attrs: &[Attribute] = match expr {
            Expr::Block(v) => &v.attrs,
            Expr::Call(v) => &v.attrs,
            Expr::MethodCall(v) => &v.attrs,
            _ => &[],
        };
        if !test_only(attrs) {
            visit::visit_expr(self, expr);
        }
    }
}

impl Inventory {
    fn load_file(&mut self, root: &Path, path: &Path, only_tests: bool) -> Result<(), String> {
        if !self.visited.insert((path.to_path_buf(), only_tests)) {
            return Ok(());
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let ast = syn::parse_file(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let only_tests = only_tests || test_only(&ast.attrs);
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let parent = path.parent().ok_or("source file has no parent")?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("non-UTF8 source name")?;
        let module_dir = if matches!(stem, "main" | "lib" | "mod") {
            parent.to_path_buf()
        } else {
            parent.join(stem)
        };
        self.items(
            root,
            &relative,
            parent,
            &module_dir,
            "",
            &ast.items,
            only_tests,
        )
    }

    fn function(
        &mut self,
        file: &str,
        scope: &str,
        signature: &syn::Signature,
        attrs: &[Attribute],
        body: &syn::Block,
        only_tests: bool,
    ) {
        let name = signature.ident.to_string();
        let owner = format!("{file}::{scope}{name}");
        if has_test_attr(attrs) {
            self.tests.insert(owner.clone());
        }
        if only_tests || test_only(attrs) {
            return;
        }
        let mut wiring = Wiring::default();
        wiring.visit_block(body);
        let renderer = name == "render"
            || name.starts_with("render_")
            || matches!(&signature.output, syn::ReturnType::Type(_, ty) if element_type(ty));
        let handler = name.starts_with("on_") || name.starts_with("capture_");
        if renderer || handler || !wiring.events.is_empty() || !wiring.controls.is_empty() {
            let current = self.functions.entry(owner.clone()).or_insert_with(|| {
                json!({
                    "renderer": renderer, "handler": handler, "events": {}, "controls": {}
                })
            });
            // Same-named platform variants share one owner and aggregate counts.
            for (key, values) in [("events", wiring.events), ("controls", wiring.controls)] {
                let target = current[key].as_object_mut().unwrap();
                for (name, count) in values {
                    let old = target.get(&name).and_then(Value::as_u64).unwrap_or(0);
                    target.insert(name, json!(old + count as u64));
                }
            }
        }
        // Track local helper functions separately, including their test attributes.
        struct Nested<'a> {
            inventory: &'a mut Inventory,
            file: &'a str,
            scope: String,
        }
        impl<'ast> Visit<'ast> for Nested<'_> {
            fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
                if !test_only(&module.attrs) {
                    let old = self.scope.clone();
                    self.scope = format!("{}{}::", old, module.ident);
                    visit::visit_item_mod(self, module);
                    self.scope = old;
                }
            }

            fn visit_expr_block(&mut self, block: &'ast syn::ExprBlock) {
                if !test_only(&block.attrs) {
                    visit::visit_expr_block(self, block);
                }
            }

            fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
                self.inventory
                    .function(self.file, &self.scope, &f.sig, &f.attrs, &f.block, false);
            }
        }
        Nested {
            inventory: self,
            file,
            scope: format!("{scope}{name}::"),
        }
        .visit_block(body);
    }

    #[allow(clippy::too_many_arguments)]
    fn items(
        &mut self,
        root: &Path,
        file: &str,
        file_parent: &Path,
        module_dir: &Path,
        scope: &str,
        items: &[Item],
        only_tests: bool,
    ) -> Result<(), String> {
        for item in items {
            match item {
                Item::Mod(module) => {
                    let only_tests = only_tests || test_only(&module.attrs);
                    let name = module.ident.to_string();
                    if let Some((_, items)) = &module.content {
                        self.items(
                            root,
                            file,
                            file_parent,
                            &module_dir.join(&name),
                            &format!("{scope}{name}::"),
                            items,
                            only_tests,
                        )?;
                    } else {
                        let explicit = module
                            .attrs
                            .iter()
                            .find(|a| a.path().is_ident("path"))
                            .and_then(|a| {
                                if let Meta::NameValue(value) = &a.meta {
                                    if let Expr::Lit(lit) = &value.value {
                                        if let syn::Lit::Str(s) = &lit.lit {
                                            return Some(s.value());
                                        }
                                    }
                                }
                                None
                            });
                        let path = if let Some(path) = explicit {
                            file_parent.join(path)
                        } else {
                            let direct = module_dir.join(format!("{name}.rs"));
                            if direct.exists() {
                                direct
                            } else {
                                module_dir.join(name).join("mod.rs")
                            }
                        };
                        self.load_file(root, &path, only_tests)?;
                    }
                }
                Item::Fn(f) => self.function(file, scope, &f.sig, &f.attrs, &f.block, only_tests),
                Item::Impl(implementation) => {
                    let owner = match &implementation.trait_ {
                        Some((_, path, _)) => format!(
                            "impl:{} for {}::",
                            path_label(path),
                            type_label(&implementation.self_ty)
                        ),
                        None => format!("impl:{}::", type_label(&implementation.self_ty)),
                    };
                    for member in &implementation.items {
                        if let syn::ImplItem::Fn(f) = member {
                            self.function(
                                file,
                                &format!("{scope}{owner}"),
                                &f.sig,
                                &f.attrs,
                                &f.block,
                                only_tests || test_only(&implementation.attrs),
                            );
                        }
                    }
                }
                Item::Trait(trait_item) => {
                    for member in &trait_item.items {
                        if let syn::TraitItem::Fn(f) = member {
                            if let Some(body) = &f.default {
                                self.function(
                                    file,
                                    &format!("{scope}trait:{}::", trait_item.ident),
                                    &f.sig,
                                    &f.attrs,
                                    body,
                                    only_tests || test_only(&trait_item.attrs),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn discover(root: &Path) -> Result<Inventory, String> {
    let mut inventory = Inventory::default();
    for entry in ["src/main.rs", "src/lib.rs"] {
        if root.join(entry).exists() {
            inventory.load_file(root, &root.join(entry), false)?;
        }
    }
    Ok(inventory)
}

fn validate(inventory: &Inventory, ledger: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if ledger["schema_version"] != json!(1) {
        errors.push("ledger schema_version must be 1".into());
    }
    let Some(entries) = ledger["entries"].as_array() else {
        return vec!["ledger entries must be an array".into()];
    };
    let mut listed = BTreeSet::new();
    for entry in entries {
        let Some(owner) = entry["owner"].as_str() else {
            errors.push("entry is missing owner".into());
            continue;
        };
        if !listed.insert(owner.to_string()) {
            errors.push(format!("duplicate owner {owner}"));
        }
        match inventory.functions.get(owner) {
            None => errors.push(format!("stale/unmapped ledger owner {owner}")),
            Some(discovered) => {
                for field in ["renderer", "handler", "events", "controls"] {
                    if entry.get(field) != discovered.get(field) {
                        errors.push(format!(
                            "changed {field} for {owner}: expected {}",
                            discovered[field]
                        ));
                    }
                }
            }
        }
        let Some(coverage) = entry["coverage"].as_array().filter(|v| !v.is_empty()) else {
            errors.push(format!("unmapped coverage for {owner}: add named regression evidence or an explicit gap/platform requirement"));
            continue;
        };
        for evidence in coverage {
            let kind = evidence["kind"].as_str().unwrap_or("");
            if !matches!(
                kind,
                "hit_tested" | "state_model_mocked" | "platform_device_required" | "gap"
            ) {
                errors.push(format!("invalid coverage kind for {owner}: {kind}"));
            }
            if evidence["note"]
                .as_str()
                .is_none_or(|s| s.trim().len() < 20 || s.contains("REVIEW REQUIRED"))
            {
                errors.push(format!(
                    "specific coverage boundary note required for {owner}"
                ));
            }
            let tests = evidence["tests"].as_array();
            if matches!(kind, "hit_tested" | "state_model_mocked")
                && tests.is_none_or(|v| v.is_empty())
            {
                errors.push(format!("{kind} for {owner} requires named tests"));
            }
            for test in tests.into_iter().flatten() {
                let name = test.as_str().unwrap_or("");
                if !inventory.tests.contains(name) {
                    errors.push(format!("stale test reference for {owner}: {name}"));
                }
            }
        }
    }
    for (owner, shape) in &inventory.functions {
        if !listed.contains(owner) {
            let mut row = shape.clone();
            row["owner"] = json!(owner);
            row["coverage"] = json!([{"kind":"gap", "tests":[], "note":"REVIEW REQUIRED: specify this owner's untested interactions and any device or service prerequisites."}]);
            errors.push(format!("unlisted production renderer/event/control owner {owner}. Add and review ledger row:\n{}", serde_json::to_string_pretty(&row).unwrap()));
        }
    }
    errors
}

#[test]
fn production_event_inventory_matches_reviewed_coverage_ledger() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inventory = discover(root).unwrap_or_else(|e| panic!("inventory discovery failed: {e}"));
    let text = std::fs::read_to_string(root.join(LEDGER)).expect("coverage ledger must exist");
    let ledger: Value = serde_json::from_str(&text).expect("coverage ledger must be valid JSON");
    let errors = validate(&inventory, &ledger);
    assert!(
        errors.is_empty(),
        "Interaction coverage inventory drift:\n{}\nRun ignored dump_interaction_coverage_inventory with --ignored --nocapture, review differences, and update {LEDGER}. Never bless unknown controls as covered.",
        errors.join("\n\n")
    );
}

#[test]
#[ignore = "read-only discovery report for reviewed ledger maintenance, not automatic blessing"]
fn dump_interaction_coverage_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inventory = discover(root).unwrap();
    let rows: Vec<_> = inventory
        .functions
        .iter()
        .map(|(owner, shape)| {
            let mut row = shape.clone();
            row["owner"] = json!(owner);
            row
        })
        .collect();
    println!(
        "INTERACTION_INVENTORY_BEGIN\n{}\nINTERACTION_INVENTORY_END",
        serde_json::to_string_pretty(
            &json!({"schema_version":1, "entries":rows, "named_tests":inventory.tests})
        )
        .unwrap()
    );
}

fn fixture(source: &str) -> Inventory {
    let ast = syn::parse_file(source).unwrap();
    let mut inventory = Inventory::default();
    inventory
        .items(
            Path::new("."),
            "src/fixture.rs",
            Path::new("src"),
            Path::new("src/fixture"),
            "",
            &ast.items,
            false,
        )
        .unwrap();
    inventory
}

fn fixture_ledger(inventory: &Inventory) -> Value {
    let rows: Vec<_> = inventory.functions.iter().map(|(owner, shape)| {
        let mut row = shape.clone();
        row["owner"] = json!(owner);
        row["coverage"] = json!([{"kind":"gap", "tests":[], "note":"Fixture owner deliberately has no real interaction regression coverage."}]);
        row
    }).collect();
    json!({"schema_version":1, "entries":rows})
}

#[test]
fn guard_rejects_new_event_wiring_and_control_families() {
    let old = fixture("fn render() { div().id(\"known\").on_click(handler); }");
    let ledger = fixture_ledger(&old);
    assert!(validate(&old, &ledger).is_empty());
    for source in [
        "fn render() { div().id(\"known\").on_click(handler).on_click(other); }",
        "fn render() { div().id(\"new-control\").on_click(handler); }",
        "fn render() { div().id(\"known\").on_click(handler); } fn render_new() {}",
        "fn render() { div().id(\"known\").on_click(handler); } fn new_surface() -> AnyElement { div() }",
    ] {
        assert!(
            !validate(&fixture(source), &ledger).is_empty(),
            "new wiring/control/renderer must require ledger review"
        );
    }
}

#[test]
fn guard_rejects_unmapped_entries_and_stale_test_references() {
    let inventory =
        fixture("fn render() { div().on_click(handler); } #[test] fn real_regression() {} ");
    let mut ledger = fixture_ledger(&inventory);
    ledger["entries"][0]["coverage"] = json!([]);
    assert!(
        validate(&inventory, &ledger)
            .iter()
            .any(|e| e.contains("unmapped coverage"))
    );
    ledger["entries"][0]["coverage"] = json!([{"kind":"hit_tested", "tests":["src/fixture.rs::real_regression"], "note":"Fixture named regression exists but no real GPUI interaction is performed here."}]);
    assert!(validate(&inventory, &ledger).is_empty());
    ledger["entries"][0]["coverage"][0]["tests"] = json!(["src/fixture.rs::deleted_test"]);
    assert!(
        validate(&inventory, &ledger)
            .iter()
            .any(|e| e.contains("stale test reference"))
    );
    ledger["entries"][0]["owner"] = json!("src/fixture.rs::not_a_real_owner");
    assert!(
        validate(&inventory, &ledger)
            .iter()
            .any(|e| e.contains("stale/unmapped ledger owner"))
    );
}

#[test]
fn discovery_skips_test_only_code_and_collects_test_attributes_syntax_aware() {
    let inventory = fixture(
        r#"
        // div().on_fake_comment(handler);
        fn production() { let _ = "div().on_fake_string(handler)"; div().on_click(handler); }
        #[cfg(test)] mod tests { fn render_fake() { div().on_fake(handler); } #[gpui::test] fn actual_gui_test() {} }
        #[cfg(all(test, target_os = "ios"))] fn test_only() { div().capture_fake(handler); }
        #[cfg(target_os = "ios")] fn render_platform() { div().on_touch(handler); }
        #[cfg(not(test))] fn render_release() { div().id(("row", id)); }
        #[cfg(any(test, target_os = "ios"))] fn render_possible_production() {}
    "#,
    );
    assert_eq!(inventory.functions.len(), 4);
    assert_eq!(
        inventory.functions["src/fixture.rs::production"]["events"],
        json!({"on_click":1})
    );
    assert!(
        inventory
            .tests
            .contains("src/fixture.rs::tests::actual_gui_test")
    );
    assert!(
        !inventory
            .functions
            .keys()
            .any(|key| key.contains("fake") || key.contains("test_only"))
    );
}
