use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, ItemFn, ItemImpl, Result};

pub fn expand(ast: &DeriveInput, name: &Ident) -> Result<TokenStream> {
    // Handle generics
    let generics = ast.generics.clone();
    let (gen_impl, gen_ty, gen_where) = generics.split_for_impl();

    // Check structure type and extracts fields
    let fields = match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(&fields.named),
            _ => Err(Error::new(name.span(), "expect struct with named fields")),
        },
        _ => Err(Error::new(name.span(), "expect struct")),
    }?;

    // Search for mandatory field based on type
    let mut properties_field = None;
    let mut port_fields = Vec::new();

    for field in fields {
        if let Some(field_name) = &field.ident {
            let type_name = crate::types::typename_with_gen(&field.ty);

            // Check for #[port] attribute
            let has_port_attr = field.attrs.iter().any(|attr| attr.path().is_ident("port"));

            if has_port_attr {
                port_fields.push(field_name);
            } else {
                // Check for required types
                if type_name.as_str() == "Arc<Properties>" {
                    if properties_field.is_some() {
                        return Err(Error::new(
                            field_name.span(),
                            "Only one Arc<Properties> field is allowed",
                        ));
                    }
                    properties_field = Some(field_name);
                }
            }
        }
    }
    let properties_field = properties_field
        .ok_or_else(|| Error::new(name.span(), "Requires exactly one Arc<Properties> field"))?;

    // Generate ports match entries
    let port_entries = port_fields.iter().map(|field_name| {
        let field_name_str = field_name.to_string();
        quote! {
            #field_name_str => &self.#field_name as &dyn Port,
        }
    });

    // Generate the implementation
    let module_impl = quote! {
        impl #gen_impl Module for #name #gen_ty #gen_where {

            /// Get module properties
            fn properties(&self) -> &Arc<Properties> {
                &self.#properties_field
            }

            /// Get module Ports
            /// Use to postponned the port binding at the end of elaboration
            /// NB: Provide default implementation for some function while trait is evolving
            ///  => prevent unnecessary break in unit tests
            fn port(&self, name: &str) -> &dyn port::Port{
                match name {
                    #(#port_entries)*
                    _ => {
                        panic!(
                            "Invalid port request {} on instance {}",
                            name,
                            self.#properties_field.path()
                        );
                    }
                }

            }

            /// This function extract sub-modules that match the given path regex
            /// Matching is done only on the first path segment
            /// For extraction of innermost node that match a path pattern, see `inner_match_recurse`
            /// function
            /// A default implementation is provided for flat Module
            fn inner_match(&self, _name: &str) -> Vec<&dyn Module> {
                Vec::new()
            }

            /// Modules without inner modules are considered as tree leaf.
            /// A default implementation is provided for flat Modules
            fn is_leaf(&self) -> bool {
                true
            }

            /// Register process in the scheduler
            fn init(self: Arc<Self>) {
                self.__module_init()
            }

            /// Abort module process to drop the associated object
            fn teardown(self: Arc<Self>) {
                self.__module_teardown();
            }
        }
    };

    Ok(module_impl)
}

pub fn expand_init(ast: &ItemFn, fn_name: &Ident) -> Result<proc_macro2::TokenStream> {
    // Verify function signature
    if ast.sig.inputs.len() != 1 {
        return Err(Error::new(
            fn_name.span(),
            "#[init] function must take exactly one argument",
        ));
    }
    if match &ast.sig.inputs[0] {
        syn::FnArg::Receiver(receiver) => {
            crate::types::typename_with_gen(&receiver.ty) != "Arc<Self>"
        }
        _ => true,
    } {
        return Err(Error::new(
            fn_name.span(),
            "#[init] function must take Arc<Self> argument",
        ));
    }

    let binding = quote! {
        // Generate the internal init function binding that Module::init will call
        fn __module_init(self: Arc<Self>) {
            self.#fn_name()
        }
    };

    Ok(quote!(
        #binding
        #ast
    ))
}

pub fn expand_default_init(ast: &ItemImpl, impl_name: &Ident) -> Result<proc_macro2::TokenStream> {
    // Handle generics
    let generics = ast.generics.clone();
    let (gen_impl, gen_ty, gen_where) = generics.split_for_impl();

    let default = quote! {
        impl #gen_impl #impl_name #gen_ty #gen_where {
            fn __module_init(self: Arc<Self>) {}
        }
    };

    Ok(quote!(
        #default
        #ast
    ))
}

pub fn expand_teardown(ast: &ItemFn, fn_name: &Ident) -> Result<proc_macro2::TokenStream> {
    // Verify function signature
    if ast.sig.inputs.len() != 1 {
        return Err(Error::new(
            fn_name.span(),
            "#[teardown] function must take exactly one argument",
        ));
    }
    if match &ast.sig.inputs[0] {
        syn::FnArg::Receiver(receiver) => {
            crate::types::typename_with_gen(&receiver.ty) != "Arc<Self>"
        }
        _ => true,
    } {
        return Err(Error::new(
            fn_name.span(),
            "#[teardown] function must take Arc<Self> argument",
        ));
    }

    let binding = quote! {
        // Generate the internal init function that Module::init will call
        fn __module_teardown(self: Arc<Self>) {
            self.#fn_name();
        }
    };

    Ok(quote!(
        #binding
        #ast
    ))
}

pub fn expand_default_teardown(
    ast: &ItemImpl,
    impl_name: &Ident,
) -> Result<proc_macro2::TokenStream> {
    // Handle generics
    let generics = ast.generics.clone();
    let (gen_impl, gen_ty, gen_where) = generics.split_for_impl();

    let default = quote! {
        impl #gen_impl #impl_name #gen_ty #gen_where {
            fn __module_teardown(self: Arc<Self>) {}
        }
    };

    Ok(quote!(
    #default
    #ast
    ))
}
