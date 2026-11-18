//! `#[derive(FexuForm)]` builds a standalone form or a model form from an allow-list.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, LitStr, Path, PathArguments, Type};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let header = form_header(&input)?;
    let name = input.ident;
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(named) => named.named,
            _ => return Err(syn::Error::new_spanned(name, "FexuForm needs named fields")),
        },
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "FexuForm only supports structs",
            ))
        }
    };
    let mut parsed = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new_spanned(&name, "FexuForm needs named fields"))?;
        let attrs = field_attrs(&field.attrs)?;
        let mut column = attrs.column.clone().unwrap_or_else(|| ident.to_string());
        if !is_field_name(&column) {
            return Err(syn::Error::new_spanned(
                &ident,
                "form column name is not an identifier",
            ));
        }
        if header.model.is_none() {
            column = ident.to_string();
        }
        parsed.push(ParsedField {
            ident,
            column,
            attrs,
            ty: field.ty,
        });
    }
    if parsed.is_empty() {
        return Err(syn::Error::new_spanned(
            &name,
            "FexuForm needs at least one field",
        ));
    }
    if let Some(model) = header.model {
        model_form(&name, &model, &header.cross, &parsed)
    } else {
        standalone_form(&name, &header.cross, &parsed)
    }
}

struct FormHeader {
    model: Option<Path>,
    cross: Option<Path>,
}

#[derive(Default)]
struct FieldAttrs {
    column: Option<String>,
    label: Option<String>,
    help: Option<String>,
    placeholder: Option<String>,
    widget: Option<String>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    choices: Option<String>,
    check: Option<Path>,
    readonly_unless: Option<String>,
    required_when: Option<String>,
    force_required: bool,
    read_only: bool,
}

struct ParsedField {
    ident: Ident,
    column: String,
    attrs: FieldAttrs,
    ty: Type,
}

fn model_form(
    name: &Ident,
    model: &Path,
    cross: &Option<Path>,
    fields: &[ParsedField],
) -> syn::Result<TokenStream> {
    let mut idents = Vec::new();
    let mut columns = Vec::new();
    let mut readers = Vec::new();
    let mut access = Vec::new();
    let mut widgets = Vec::new();
    for field in fields {
        let reader = read_model_field(&field.ident, &field.column, &field.ty)?;
        let column = &field.column;
        let label = field.attrs.label.clone().unwrap_or_else(|| column.clone());
        let help = field.attrs.help.clone().unwrap_or_default();
        let placeholder = field.attrs.placeholder.clone().unwrap_or_default();
        let widget = match widget_tokens(field.attrs.widget.as_deref())? {
            Some(kind) => quote! { ::core::option::Option::Some(#kind) },
            None => quote! { ::core::option::Option::None },
        };
        let readonly_unless = opt_str(field.attrs.readonly_unless.as_deref());
        let required_when = opt_str(field.attrs.required_when.as_deref());
        if field.attrs.read_only {
            return Err(syn::Error::new_spanned(
                &field.ident,
                "a model field uses readonly_unless; read-only model columns stay out of the allow-list",
            ));
        }
        access.push(quote! {
            ::fexu::forms::FieldAccess {
                name: #column,
                readonly_unless: #readonly_unless,
                required_when: #required_when,
            }
        });
        widgets.push(quote! {
            {
                let Some(spec) = fields.iter().find(|field| field.name == #column) else {
                    return ::core::result::Result::Err(::fexu::forms::FormSetupError {
                        code: "form.unknown_column",
                    });
                };
                let mut widget = ::fexu::forms::model_widget(spec, #label);
                widget.help = #help;
                widget.placeholder = #placeholder;
                if let ::core::option::Option::Some(kind) = #widget {
                    widget.widget = kind;
                }
                rendered.push(widget);
            }
        });
        idents.push(&field.ident);
        columns.push(column.clone());
        readers.push(reader);
    }
    let cross_call = cross_call(cross);
    Ok(quote! {
        impl #name {
            pub fn from_submit(
                mode: ::fexu::forms::SubmitMode,
                values: &::fexu::forms::FormValues,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                let context = ::fexu::forms::FormContext { mode, flags: &[] };
                Self::from_submit_in(mode, values, &context)
            }

            pub fn from_submit_in(
                mode: ::fexu::forms::SubmitMode,
                values: &::fexu::forms::FormValues,
                context: &::fexu::forms::FormContext<'_>,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                Self::from_submit_in_zone(mode, values, context, ::core::option::Option::None)
            }

            pub fn from_submit_in_zone(
                mode: ::fexu::forms::SubmitMode,
                values: &::fexu::forms::FormValues,
                context: &::fexu::forms::FormContext<'_>,
                zone: ::core::option::Option<::fexu::forms::Zone>,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                let allow: &[&str] = &[#(#columns),*];
                let admin = <#model as ::fexu::core::FexuModel>::admin_meta()
                    .map(|meta| meta.fields)
                    .unwrap_or(&[]);
                let mut fields = match ::fexu::forms::model_fields(
                    <#model as ::fexu::core::FexuModel>::columns(),
                    admin,
                    allow,
                ) {
                    ::core::result::Result::Ok(fields) => fields,
                    ::core::result::Result::Err(error) => {
                        let mut errors = ::fexu::forms::FormErrors::default();
                        errors.push("", error.code);
                        return ::core::result::Result::Err(errors);
                    }
                };
                ::fexu::forms::apply_bounds(
                    &mut fields,
                    <#model as ::fexu::core::FexuModel>::numeric_bounds(),
                );
                let access = [#(#access),*];
                let prepared;
                let values = if let ::core::option::Option::Some(zone) = zone {
                    match ::fexu::forms::prepare_model_datetimes(&fields, values, zone) {
                        ::core::result::Result::Ok(next) => {
                            prepared = next;
                            &prepared
                        }
                        ::core::result::Result::Err(errors) => {
                            return ::core::result::Result::Err(errors);
                        }
                    }
                } else {
                    values
                };
                let bound = ::fexu::forms::bind_in_context(
                    mode,
                    &fields,
                    values,
                    &[],
                    &[],
                    &access,
                    context,
                )?;
                let built = Self { #(#readers,)* };
                #cross_call
                ::core::result::Result::Ok(built)
            }

            pub fn widgets() -> ::core::result::Result<
                ::std::vec::Vec<::fexu::forms::Field>,
                ::fexu::forms::FormSetupError,
            > {
                let allow: &[&str] = &[#(#columns),*];
                let admin = <#model as ::fexu::core::FexuModel>::admin_meta()
                    .map(|meta| meta.fields)
                    .unwrap_or(&[]);
                let fields = ::fexu::forms::model_fields(
                    <#model as ::fexu::core::FexuModel>::columns(),
                    admin,
                    allow,
                )?;
                let mut rendered = ::std::vec::Vec::new();
                #(#widgets)*
                ::core::result::Result::Ok(rendered)
            }
        }

        impl ::fexu::forms::FexuForm for #name {
            fn from_form(
                values: &::fexu::forms::FormValues,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                Self::from_submit(::fexu::forms::SubmitMode::Create, values)
            }
        }
    })
}

fn standalone_form(
    name: &Ident,
    cross: &Option<Path>,
    fields: &[ParsedField],
) -> syn::Result<TokenStream> {
    let mut declared = Vec::new();
    let mut readers = Vec::new();
    for field in fields {
        let kind = standalone_kind(&field.ty).ok_or_else(|| {
            syn::Error::new_spanned(
                &field.ty,
                "standalone FexuForm fields must be String, i64, bool, Vec<String>, or Option of String, i64, or bool",
            )
        })?;
        let column = &field.column;
        let label = field.attrs.label.clone().unwrap_or_else(|| column.clone());
        let help = field.attrs.help.clone().unwrap_or_default();
        let placeholder = field.attrs.placeholder.clone().unwrap_or_default();
        let widget = widget_tokens(field.attrs.widget.as_deref())?
            .unwrap_or_else(|| quote! { ::fexu::forms::Widget::Text });
        let input_kind = input_kind_tokens(kind);
        let force_required = field.attrs.force_required;
        let read_only = field.attrs.read_only;
        let readonly_unless = opt_str(field.attrs.readonly_unless.as_deref());
        let required_when = opt_str(field.attrs.required_when.as_deref());
        let min_length = opt_usize(field.attrs.min_length);
        let max_length = opt_usize(field.attrs.max_length);
        let choices = choice_tokens(field.attrs.choices.as_deref());
        let check = match &field.attrs.check {
            Some(path) => quote! { ::core::option::Option::Some(#path) },
            None => quote! { ::core::option::Option::None },
        };
        declared.push(quote! {
            ::fexu::forms::DeclaredField {
                name: #column,
                label: #label,
                widget: #widget,
                help: #help,
                placeholder: #placeholder,
                kind: #input_kind,
                force_required: #force_required,
                read_only: #read_only,
                readonly_unless: #readonly_unless,
                required_when: #required_when,
                min_length: #min_length,
                max_length: #max_length,
                choices: #choices,
                check: #check,
            }
        });
        readers.push(read_clean_field(&field.ident, column, kind)?);
    }
    let cross_call = cross_call(cross);
    Ok(quote! {
        impl #name {
            pub fn declared() -> ::std::vec::Vec<::fexu::forms::DeclaredField> {
                ::std::vec![#(#declared),*]
            }

            pub fn from_form_in(
                values: &::fexu::forms::FormValues,
                context: &::fexu::forms::FormContext<'_>,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                let clean = ::fexu::forms::bind_declared(&Self::declared(), values, context)?;
                let built = Self { #(#readers),* };
                #cross_call
                ::core::result::Result::Ok(built)
            }

            pub fn widgets() -> ::std::vec::Vec<::fexu::forms::Field> {
                let mut rendered = ::std::vec::Vec::new();
                for field in Self::declared() {
                    let required = matches!(
                        field.kind,
                        ::fexu::forms::InputKind::Text
                            | ::fexu::forms::InputKind::Integer
                            | ::fexu::forms::InputKind::Boolean
                    ) || field.force_required;
                    rendered.push(
                        ::fexu::forms::Field::new(field.name, field.label, field.widget, required)
                            .help(field.help)
                            .placeholder(field.placeholder),
                    );
                }
                rendered
            }
        }

        impl ::fexu::forms::FexuForm for #name {
            fn from_form(
                values: &::fexu::forms::FormValues,
            ) -> ::core::result::Result<Self, ::fexu::forms::FormErrors> {
                Self::from_form_in(values, &::fexu::forms::FormContext::create())
            }
        }
    })
}

fn cross_call(cross: &Option<Path>) -> TokenStream {
    match cross {
        Some(path) => quote! { #path(&built)?; },
        None => quote! {},
    }
}

fn form_header(input: &DeriveInput) -> syn::Result<FormHeader> {
    let mut header = FormHeader {
        model: None,
        cross: None,
    };
    for attr in &input.attrs {
        if !attr.path().is_ident("form") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("model") {
                header.model = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("cross") {
                header.cross = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported form attribute"))
            }
        })?;
    }
    Ok(header)
}

fn field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut parsed = FieldAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("form") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                parsed.column = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("label") {
                parsed.label = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("help") {
                parsed.help = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("placeholder") {
                parsed.placeholder = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("widget") {
                parsed.widget = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("min_length") {
                parsed.min_length = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("max_length") {
                parsed.max_length = Some(meta.value()?.parse::<syn::LitInt>()?.base10_parse()?);
            } else if meta.path.is_ident("choices") {
                parsed.choices = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("check") {
                parsed.check = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("readonly_unless") {
                parsed.readonly_unless = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("required_when") {
                parsed.required_when = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("required") {
                parsed.force_required = true;
            } else if meta.path.is_ident("readonly") {
                parsed.read_only = true;
            } else {
                return Err(meta.error("unsupported form field attribute"));
            }
            Ok(())
        })?;
    }
    Ok(parsed)
}

fn widget_tokens(name: Option<&str>) -> syn::Result<Option<TokenStream>> {
    let Some(name) = name else {
        return Ok(None);
    };
    let widget = match name {
        "text" => quote! { ::fexu::forms::Widget::Text },
        "email" => quote! { ::fexu::forms::Widget::Email },
        "password" => quote! { ::fexu::forms::Widget::Password },
        "number" => quote! { ::fexu::forms::Widget::Number },
        "url" => quote! { ::fexu::forms::Widget::Url },
        "checkbox" => quote! { ::fexu::forms::Widget::Checkbox },
        "textarea" => quote! { ::fexu::forms::Widget::Textarea },
        "select" => quote! { ::fexu::forms::Widget::Select },
        "select_multiple" => quote! { ::fexu::forms::Widget::SelectMultiple },
        "file" => quote! { ::fexu::forms::Widget::File },
        "relation" => quote! { ::fexu::forms::Widget::Relation },
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "unknown form widget",
            ))
        }
    };
    Ok(Some(widget))
}

fn opt_str(value: Option<&str>) -> TokenStream {
    match value {
        Some(value) => quote! { ::core::option::Option::Some(#value) },
        None => quote! { ::core::option::Option::None },
    }
}

fn opt_usize(value: Option<u32>) -> TokenStream {
    match value {
        Some(value) => quote! { ::core::option::Option::Some(#value as usize) },
        None => quote! { ::core::option::Option::None },
    }
}

fn choice_tokens(value: Option<&str>) -> TokenStream {
    let Some(value) = value else {
        return quote! { &[] };
    };
    let parts: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    quote! { &[#(#parts),*] }
}

fn input_kind_tokens(kind: StandaloneKind) -> TokenStream {
    match kind {
        StandaloneKind::Text => quote! { ::fexu::forms::InputKind::Text },
        StandaloneKind::OptionalText => quote! { ::fexu::forms::InputKind::OptionalText },
        StandaloneKind::Integer => quote! { ::fexu::forms::InputKind::Integer },
        StandaloneKind::OptionalInteger => quote! { ::fexu::forms::InputKind::OptionalInteger },
        StandaloneKind::Boolean => quote! { ::fexu::forms::InputKind::Boolean },
        StandaloneKind::OptionalBoolean => quote! { ::fexu::forms::InputKind::OptionalBoolean },
        StandaloneKind::Texts => quote! { ::fexu::forms::InputKind::Texts },
    }
}

fn read_model_field(ident: &Ident, column: &str, ty: &Type) -> syn::Result<TokenStream> {
    let kind = match type_kind(ty) {
        Some(kind) => kind,
        None => {
            return Err(syn::Error::new_spanned(
                ty,
                "FexuForm fields must be String, i64, bool, or Option of those",
            ))
        }
    };
    let value = format_ident!("value");
    let body = match kind {
        FieldKind::String => quote! {
            match #value {
                ::fexu::forms::BoundValue::Text(text) => text.clone(),
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        FieldKind::Integer => quote! {
            match #value {
                ::fexu::forms::BoundValue::Integer(number) => *number,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        FieldKind::Boolean => quote! {
            match #value {
                ::fexu::forms::BoundValue::Boolean(flag) => *flag,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        FieldKind::OptionalString => quote! {
            match #value {
                ::fexu::forms::BoundValue::Text(text) => ::core::option::Option::Some(text.clone()),
                ::fexu::forms::BoundValue::Null | ::fexu::forms::BoundValue::Unchanged => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        FieldKind::OptionalInteger => quote! {
            match #value {
                ::fexu::forms::BoundValue::Integer(number) => ::core::option::Option::Some(*number),
                ::fexu::forms::BoundValue::Null | ::fexu::forms::BoundValue::Unchanged => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        FieldKind::OptionalBoolean => quote! {
            match #value {
                ::fexu::forms::BoundValue::Boolean(flag) => ::core::option::Option::Some(*flag),
                ::fexu::forms::BoundValue::Null | ::fexu::forms::BoundValue::Unchanged => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
    };
    Ok(quote! {
        #ident: {
            fn invalid(name: &str) -> ::fexu::forms::FormErrors {
                let mut errors = ::fexu::forms::FormErrors::default();
                errors.push(name, "validation.invalid_type");
                errors
            }
            let #value = bound.iter().find(|field| field.name == #column).map(|field| &field.value);
            let Some(#value) = #value else {
                return ::core::result::Result::Err(invalid(#column));
            };
            #body
        }
    })
}

fn read_clean_field(ident: &Ident, column: &str, kind: StandaloneKind) -> syn::Result<TokenStream> {
    let body = match kind {
        StandaloneKind::Text => quote! {
            match &value.value {
                ::fexu::forms::CleanValue::Text(text) => text.clone(),
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::OptionalText => quote! {
            match &value.value {
                ::fexu::forms::CleanValue::Text(text) => ::core::option::Option::Some(text.clone()),
                ::fexu::forms::CleanValue::Empty | ::fexu::forms::CleanValue::Absent => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::Integer => quote! {
            match value.value {
                ::fexu::forms::CleanValue::Integer(number) => number,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::OptionalInteger => quote! {
            match value.value {
                ::fexu::forms::CleanValue::Integer(number) => ::core::option::Option::Some(number),
                ::fexu::forms::CleanValue::Empty | ::fexu::forms::CleanValue::Absent => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::Boolean => quote! {
            match value.value {
                ::fexu::forms::CleanValue::Boolean(flag) => flag,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::OptionalBoolean => quote! {
            match value.value {
                ::fexu::forms::CleanValue::Boolean(flag) => ::core::option::Option::Some(flag),
                ::fexu::forms::CleanValue::Empty | ::fexu::forms::CleanValue::Absent => ::core::option::Option::None,
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
        StandaloneKind::Texts => quote! {
            match &value.value {
                ::fexu::forms::CleanValue::Texts(items) => items.clone(),
                ::fexu::forms::CleanValue::Empty | ::fexu::forms::CleanValue::Absent => ::std::vec::Vec::new(),
                _ => return ::core::result::Result::Err(invalid(#column)),
            }
        },
    };
    Ok(quote! {
        #ident: {
            fn invalid(name: &str) -> ::fexu::forms::FormErrors {
                let mut errors = ::fexu::forms::FormErrors::default();
                errors.push(name, "validation.invalid_type");
                errors
            }
            let Some(value) = clean.iter().find(|field| field.name == #column) else {
                return ::core::result::Result::Err(invalid(#column));
            };
            #body
        }
    })
}

enum FieldKind {
    String,
    Integer,
    Boolean,
    OptionalString,
    OptionalInteger,
    OptionalBoolean,
}

#[derive(Clone, Copy)]
enum StandaloneKind {
    Text,
    OptionalText,
    Integer,
    OptionalInteger,
    Boolean,
    OptionalBoolean,
    Texts,
}

fn standalone_kind(ty: &Type) -> Option<StandaloneKind> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    match segment.ident.to_string().as_str() {
        "String" => Some(StandaloneKind::Text),
        "i64" => Some(StandaloneKind::Integer),
        "bool" => Some(StandaloneKind::Boolean),
        "Vec" => {
            let inner = angle_inner(segment)?;
            match standalone_kind(inner)? {
                StandaloneKind::Text => Some(StandaloneKind::Texts),
                _ => None,
            }
        }
        "Option" => {
            let inner = angle_inner(segment)?;
            match standalone_kind(inner)? {
                StandaloneKind::Text => Some(StandaloneKind::OptionalText),
                StandaloneKind::Integer => Some(StandaloneKind::OptionalInteger),
                StandaloneKind::Boolean => Some(StandaloneKind::OptionalBoolean),
                _ => None,
            }
        }
        _ => None,
    }
}

fn angle_inner(segment: &syn::PathSegment) -> Option<&Type> {
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let inner = args.args.first()?;
    let syn::GenericArgument::Type(inner) = inner else {
        return None;
    };
    Some(inner)
}

fn type_kind(ty: &Type) -> Option<FieldKind> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    match segment.ident.to_string().as_str() {
        "String" => Some(FieldKind::String),
        "i64" => Some(FieldKind::Integer),
        "bool" => Some(FieldKind::Boolean),
        "Option" => {
            let inner = angle_inner(segment)?;
            match type_kind(inner)? {
                FieldKind::String => Some(FieldKind::OptionalString),
                FieldKind::Integer => Some(FieldKind::OptionalInteger),
                FieldKind::Boolean => Some(FieldKind::OptionalBoolean),
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(char) if char.is_ascii_alphabetic() || char == '_')
        && chars.all(|char| char.is_ascii_alphanumeric() || char == '_')
        && name.len() <= 64
}
