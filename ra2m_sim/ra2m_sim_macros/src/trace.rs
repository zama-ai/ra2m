use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{
    parse_quote, Attribute, Data, DeriveInput, Error, Field, Fields, ItemStruct, Result, Type,
};

pub fn expand(ast: &DeriveInput, name: &Ident) -> Result<TokenStream> {
    // Handle generics
    let generics = ast.generics.clone();
    let (gen_impl, gen_ty, gen_where) = generics.split_for_impl();

    // Enforce that where clause exist and extend it
    let mut gen_where = gen_where.cloned();
    gen_where.get_or_insert_with(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });

    // Add a new predicate to it:
    gen_where
        .as_mut()
        .unwrap()
        .predicates
        .push(syn::parse_quote!(#name #gen_ty: serde::Serialize));

    // extract #[history(...)] attribute
    let history_field_ident = extract_history_field(&ast.attrs, name)?;

    // extract #[trace_custom(...)] attribute
    let custom_type_name = extract_custom_type(&ast.attrs, name).unwrap_or(parse_quote!(()));

    // extract list of field marked as #[traced]
    let fields = match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(&fields.named),
            _ => Err(Error::new(name.span(), "expect struct with named fields")),
        },
        _ => Err(Error::new(name.span(), "expect struct")),
    }?;
    let traced_fields: Vec<_> = fields
        .iter()
        .filter(|f| f.attrs.iter().any(|attr| attr.path().is_ident("trace")))
        .collect();

    // Generate extraction code for each traced field
    let traced_extract = traced_fields.iter().map(|f| {
        let ident = f.ident.as_ref().unwrap();
        quote! {
            x.#ident.clone(),
        }
    });

    // Construct column named fixed name extend with traced one
    let mut column_name = vec![
        "transaction_id".to_string(),
        "component_id".to_string(),
        "span_start".to_string(),
        "span_stop".to_string(),
        "span_dur".to_string(),
    ];
    for f in traced_fields.iter() {
        let ident = f.ident.as_ref().unwrap();
        let name = ident.to_string();
        column_name.push(name)
    }

    let df_construct = column_name.iter().enumerate().map(|(pos, name)| {
        let idx = syn::Index::from(pos);
        quote! {
        (#name, vec_tuple.iter().map(|v| (&(v.#idx)).into()).collect::<Vec<Traceable>>()),
        }
    });

    let expanded = quote! {
        impl #gen_impl Trace for #name #gen_ty #gen_where {
            type Custom = #custom_type_name;

            fn get_history(&self) -> &types::History<Self::Custom> {
                &self.#history_field_ident
            }

            fn get_history_mut(&mut self) -> &mut types::History<Self::Custom> {
                &mut self.#history_field_ident
            }

            fn append_handler(&mut self, handler: types::Handler<Self::Custom>) {
                self.#history_field_ident.push(handler)
            }
            fn wrap_up(&mut self, uid: usize) {
                self.#history_field_ident.push(types::Handler::base(uid))
            }
            fn export_as_traceable_map(vec: &[Self]) -> std::collections::HashMap<&'static str, Vec<Traceable>>{
                // convert Vec<Struct> in Vec<Tuple> where spans were expanded and traced field inserted
                let vec_tuple = vec.iter().enumerate().flat_map(|(id, x)| {
                    let spans = x.get_history().spans();
                    spans.iter().map(|s| {
                        (id as u64,
                         *s.0 as u64,
                         s.1.start as u64,
                         s.1.end as u64,
                         s.1.duration() as u64,
                        #(#traced_extract)*)
                    }).collect::<Vec<_>>()
                }).collect::<Vec<_>>();

                std::collections::HashMap::from([
                    #(#df_construct)*
                    ])
            }
        }
    };

    Ok(expanded)
}

fn extract_history_field(attrs: &[Attribute], name: &Ident) -> Result<Ident> {
    for attr in attrs {
        if attr.path().is_ident("history") {
            let ident: Ident = attr.parse_args().expect("Expected #[history(field)]");
            return Ok(ident);
        }
    }
    Err(Error::new(
        name.span(),
        "#[derive(Trace)] expect `history(_)` or `default_history` attributes",
    ))
}

fn extract_custom_type(attrs: &[Attribute], name: &Ident) -> Result<Type> {
    for attr in attrs {
        if attr.path().is_ident("trace_custom") {
            let cust_type: Type = attr
                .parse_args()
                .expect("Expected #[trace_custom(T: Type)]");
            return Ok(cust_type);
        }
    }
    Err(Error::new(
        name.span(),
        "#[derive(Trace)] expect `custom_trace(_)` attribute",
    ))
}

pub fn expand_default_history(
    ast: &mut ItemStruct,
    name: &Ident,
) -> Result<proc_macro2::TokenStream> {
    // Create default history field
    let history_field = Field {
        attrs: vec![],
        vis: syn::parse_quote!(pub),
        mutability: syn::FieldMutability::None,
        ident: Some(Ident::new("history", Span::call_site())),
        colon_token: Some(Default::default()),
        ty: Type::Path(syn::parse_quote!(String)),
    };

    // Check structure type and append field
    match &mut ast.fields {
        Fields::Named(fields) => {
            fields.named.push(history_field);
        }
        _ => return Err(Error::new(name.span(), "expect struct with named fields")),
    }

    // Leave a history marker behind with default name
    let history_marker: syn::Attribute = syn::parse_quote!(#[history(history)]);
    ast.attrs.push(history_marker);

    // Emit the code
    Ok(quote! {#ast })
}
