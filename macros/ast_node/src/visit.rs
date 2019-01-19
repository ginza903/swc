use darling::FromField;
use swc_macros_common::prelude::*;

#[derive(Debug, FromField)]
#[darling(attributes(fold))]
struct FieldAttrs {
    ///
    #[darling(default)]
    pub ignore: bool,

    /// Should we add bound for the field's type?
    #[darling(default)]
    pub bound: bool,
}

pub fn derive(input: DeriveInput) -> ItemImpl {
    let mut derive_generics = Derive::new(&input);

    let preds = derive_generics
        .all_generic_fields()
        .into_iter()
        .filter(|f| {
            f.attrs
                .iter()
                .any(|attr| is_attr_name(attr, "fold") && attr.tts.to_string() == "( bound )")
        })
        .map(|f| f.ty.clone())
        .map(normalize_type_for_bound)
        .map(|ty| {
            Quote::new(def_site::<Span>())
                .quote_with(smart_quote!(
                    Vars { Type: &ty },
                    (Type: swc_common::VisitWith<__V>)
                ))
                .parse()
        });
    derive_generics.add_where_predicates(preds);

    let arms = Binder::new_from(&input)
        .variants()
        .into_iter()
        .map(|v| {
            let (pat, bindings) = v.bind("_", Some(def_site()), None);

            let fields: Punctuated<Stmt, token::Semi> = bindings
                .into_iter()
                .filter_map(|binding| {
                    // This closure will not be called for unit-like struct.

                    let value = match should_skip_field(binding.field()) {
                        true => None,
                        false => Some(
                            Quote::new(def_site::<Span>())
                                .quote_with(smart_quote!(
                                    Vars {
                                        FieldType: &binding.field().ty,
                                        binded_field: binding.name(),
                                    },
                                    {
                                        swc_common::Visit::<FieldType>::visit(_v, binded_field);
                                    }
                                ))
                                .parse::<Stmt>(),
                        ),
                    };

                    let _attrs = binding
                        .field()
                        .attrs
                        .iter()
                        .filter(|attr| is_attr_name(attr, "cfg"))
                        .cloned()
                        .collect::<Vec<_>>();

                    value
                })
                .map(|t| Element::Punctuated(t, def_site()))
                .collect();

            let body = match *v.data() {
                // Handle unit-like structs separately
                Fields::Unit => Box::new(
                    Quote::new(def_site::<Span>())
                        .quote_with(smart_quote!(Vars {}, {
                            {
                                // no-op
                            }
                        }))
                        .parse(),
                ),
                _ => Box::new(
                    Quote::new(def_site::<Span>())
                        .quote_with(smart_quote!(Vars { fields }, {
                            {
                                fields
                            }
                        }))
                        .parse(),
                ),
            };

            Arm {
                body,

                attrs: v
                    .attrs()
                    .iter()
                    .filter(|attr| is_attr_name(attr, "cfg"))
                    .cloned()
                    .collect(),
                pats: vec![Element::End(pat)].into_iter().collect(),
                guard: None,
                fat_arrow_token: def_site(),
                comma: Some(def_site()),
                leading_vert: None,
            }
        })
        .collect();

    let body = Expr::Match(ExprMatch {
        attrs: Default::default(),
        match_token: def_site(),
        brace_token: def_site(),
        expr: Box::new(
            Quote::new(def_site::<Span>())
                .quote_with(smart_quote!(Vars {}, { *self }))
                .parse(),
        ),
        arms,
    });

    let item = Quote::new(def_site::<Span>())
        .quote_with(smart_quote!(
            Vars {
                Type: &input.ident,
                body,
            },
            {
                impl<__V> swc_common::VisitWith<__V> for Type {
                    #[inline]
                    fn visit_children(&self, _v: &mut __V) {
                        body
                    }
                }
            }
        ))
        .parse();
    let item = derive_generics.append_to(item);

    // println!("Expaned:\n {}\n\n", item.dump());

    item
}

fn should_skip_field(field: &Field) -> bool {
    let attrs = FieldAttrs::from_field(field).expect("#[derive(Fold)]: failed to parse attribute");
    if attrs.ignore {
        return true;
    }

    let ty_str = field.ty.dump().to_string();
    match &*ty_str {
        "bool" | "usize" | "u128" | "u64" | "u32" | "u16" | "u8" | "isize" | "i128" | "i64"
        | "i32" | "i16" | "i8" | "f64" | "f32" | "String" => return true,
        _ => {}
    }

    false
}

fn normalize_type_for_bound(ty: Type) -> Type {
    use syn::fold::{self, Fold};

    struct Norm;
    impl Fold for Norm {
        fn fold_path(&mut self, path: Path) -> Path {
            if path.segments.len() == 1 {
                let seg = &path.segments[0];
                if seg.ident != "Box" && seg.ident != "Option" && seg.ident != "Vec" {
                    return path.clone();
                }

                if let PathArguments::AngleBracketed(ref args) = seg.arguments {
                    if args.args.len() == 1 {
                        if let GenericArgument::Type(ref ty) =
                            *args.args.last().unwrap().into_value()
                        {
                            match *ty {
                                Type::Path(TypePath { ref path, .. }) => {
                                    return self.fold_path(path.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            fold::fold_path(self, path)
        }
    }

    let out = Norm.fold_type(ty);
    out
}
