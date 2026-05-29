//! A set of helper function to reason about type
//!

// Helper function to extract type name from syn::Type
pub(crate) fn typename(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                segment.ident.to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

// Helper function to extract type name from syn::Type with generics support
// It handle generic nested type such as Arc<T>, Box<T>, etc...
pub(crate) fn typename_with_gen(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();

                // Handle generic types nested type
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(syn::Type::Path(inner_path))) =
                        args.args.first()
                    {
                        if let Some(inner_segment) = inner_path.path.segments.last() {
                            let inner_type_name = inner_segment.ident.to_string();
                            return format!("{}<{}>", type_name, inner_type_name);
                        }
                    }
                }
                type_name
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}
