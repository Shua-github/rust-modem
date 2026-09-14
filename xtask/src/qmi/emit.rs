//! Emits the generated Rust code as a token stream.

use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};

use crate::qmi::model::*;

/// Composite types the database repeats with the same shape in almost every
/// message. They are emitted once into `types/common.rs` and only referenced
/// from the messages, which keeps hundreds of copies out of the tree.
pub const SHARED: &[&str] = &["OperationResult"];

/// Derives a shared composite needs on top of the generated defaults.
///
/// `OperationResult` travels inside the runtime's `Error`, which is `Copy` and
/// `Eq`, so the `Result` TLV has to be both too. Every other composite is a
/// wire type that is only ever built, encoded and read back.
const EXTRA_DERIVES: &[(&str, &str)] = &[("OperationResult", "Copy, Eq")];

/// Rust name of the mandatory `Result` TLV every response carries.
const RESULT: &str = "result";

/// Whether `name` is one of the types every service shares.
fn shared(name: &str) -> bool {
    SHARED.contains(&name)
}

/// The extra derives `name` needs, ready to append to a `#[derive(..)]`.
fn extra_derives(name: &str) -> TokenStream {
    let derives = EXTRA_DERIVES
        .iter()
        .filter(|(composite, _)| *composite == name)
        .flat_map(|(_, derives)| derives.split(", "))
        .map(|derive| format_ident!("{derive}"));

    quote! { #(, #derives)* }
}

/// The composites of `service` that `types/common.rs` has to define.
pub fn shared_composites(service: &Service) -> Vec<&Composite> {
    service
        .messages
        .iter()
        .flat_map(|message| message.types.iter())
        .filter(|composite| shared(&composite.name))
        .collect()
}

/// Whether the emitted code runs in a container method, where `reader` and
/// `writer` are locals, or in a composite method, where they are `&mut`
/// parameters that need an explicit reborrow to be handed down.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Container,
    Composite,
}

impl Ctx {
    fn writer(self) -> TokenStream {
        match self {
            Ctx::Container => quote! { &mut writer },
            Ctx::Composite => quote! { writer },
        }
    }

    fn reader(self) -> TokenStream {
        match self {
            Ctx::Container => quote! { &mut reader },
            Ctx::Composite => quote! { reader },
        }
    }
}

/// Emit the body of the module of one service.
pub fn module_body(service: &Service) -> TokenStream {
    let id = Literal::u16_unsuffixed(service.id);

    let messages =
        (!service.messages.is_empty()).then(|| emit_messages("messages", &service.messages, false));
    let indications = (!service.indications.is_empty())
        .then(|| emit_messages("indications", &service.indications, true));

    quote! {
        #![allow(unused_imports)]

        pub const SERVICE_ID: u16 = #id;

        #messages
        #indications
    }
}

fn emit_messages(module: &str, messages: &[MessageDef], is_indication: bool) -> TokenStream {
    let module = format_ident!("{module}");

    let messages = messages.iter().map(|message| {
        let doc = &message.name;
        let name = format_ident!("{}", message.module);
        let id = Literal::u16_unsuffixed(message.id);

        let request = if is_indication {
            "Indication"
        } else {
            "Request"
        };
        let request = emit_container(request, &message.input, false);
        let response = (!is_indication).then(|| emit_container("Response", &message.output, true));
        let types = message
            .types
            .iter()
            .filter(|composite| !shared(&composite.name))
            .map(emit_composite);

        // A message and its response are bound to each other, so the same
        // pair serves the client that sends it and the server that answers.
        let binding = (!is_indication).then(|| {
            quote! {
                impl crate::Request for Request {
                    const SERVICE: u16 = super::super::SERVICE_ID;
                    const MESSAGE_ID: u16 = #id;
                    type Response = Response;

                    fn encode(&self) -> Result<Vec<TlvBuf>, Error> {
                        self.to_tlvs()
                    }

                    fn decode(tlvs: &[TlvBuf]) -> Result<Self, Error> {
                        Self::from_tlvs(tlvs)
                    }
                }
            }
        });

        quote! {
            #[doc = #doc]
            pub mod #name {
                use alloc::string::String;
                use alloc::vec::Vec;
                use crate::{Endian, Error, TlvBuf, TlvReader, TlvWriter};

                pub const MESSAGE_ID: u16 = #id;

                #request
                #response
                #binding
                #(#types)*
            }
        }
    });

    quote! {
        pub mod #module {
            #(#messages)*
        }
    }
}

fn emit_container(name: &str, fields: &[RField], response: bool) -> TokenStream {
    // A response carries the shared `Result` TLV, which splits its fields into
    // the data of a success and the TLVs a failure carries.
    if fields.iter().any(|field| field.is_result) {
        return emit_response(fields);
    }

    let name = format_ident!("{name}");

    // A response the database gives no `Result` TLV has no error to answer
    // with.
    let answer = response.then(|| {
        quote! {
            impl crate::Response for #name {
                type Request = Request;

                fn encode(&self) -> Result<Vec<TlvBuf>, Error> {
                    self.to_tlvs()
                }

                fn decode(tlvs: &[TlvBuf]) -> Result<Self, Error> {
                    Self::from_tlvs(tlvs)
                }
            }
        }
    });

    let members = fields.iter().map(|field| {
        let member = format_ident!("{}", field.rust_name);
        let ty = field_ty(field);
        quote! { pub #member: #ty, }
    });

    let consts = fields.iter().map(|field| {
        let const_name = format_ident!("{}", field.const_name);
        let id = Literal::u8_unsuffixed(field.tlv_id);
        quote! { pub const #const_name: u8 = #id; }
    });

    let to_tlvs = emit_to_tlvs(fields);
    let from_tlvs = emit_from_tlvs(fields);

    quote! {
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct #name {
            #(#members)*
        }

        impl #name {
            #(#consts)*
            #to_tlvs
            #from_tlvs
        }

        #answer
    }
}

/// Emit a response as the `Result` the modem answered with.
///
/// The `Result` TLV is decoded once: a zero `Error Status` means the rest of
/// the answer is the data, and a non-zero one means it is the error. Fields
/// guarded by the success of the request lose their `Option` when the database
/// itself marks them mandatory, because the arm already carries the guard.
fn emit_response(fields: &[RField]) -> TokenStream {
    let data = data_fields(fields);
    let errors = error_fields(fields);

    let consts = fields.iter().map(|field| {
        let const_name = format_ident!("{}", field.const_name);
        let id = Literal::u8_unsuffixed(field.tlv_id);
        quote! { pub const #const_name: u8 = #id; }
    });

    let members = data.iter().map(|field| {
        let member = format_ident!("{}", field.rust_name);
        let ty = field_ty(field);
        quote! { pub #member: #ty, }
    });

    let to_tlvs = emit_to_tlvs(&data);
    let from_tlvs = emit_from_tlvs(&data);

    // The error of a response is its `Result` TLV, and the TLVs the database
    // gives a failure of its own are carried beside it.
    let error = (!errors.is_empty()).then(|| emit_response_error(&errors));

    quote! {
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct Response {
            #(#members)*
        }

        impl Response {
            #(#consts)*
            #to_tlvs
            #from_tlvs
        }

        #error

        impl crate::Response for Response {
            type Request = Request;

            fn encode(&self) -> Result<Vec<TlvBuf>, Error> {
                let mut tlvs = crate::result::success_tlvs();
                tlvs.extend(self.to_tlvs()?);
                Ok(tlvs)
            }

            fn decode(tlvs: &[TlvBuf]) -> Result<Self, Error> {
                crate::result::check_result(tlvs)?;
                Response::from_tlvs(tlvs)
            }
        }
    }
}

/// Emit the error of a response whose database gives a failure TLVs of its
/// own: the `Result` TLV, with those TLVs beside it.
fn emit_response_error(errors: &[RField]) -> TokenStream {
    let members = errors.iter().map(|field| {
        let member = format_ident!("{}", field.rust_name);
        let ty = field_ty(field);
        quote! { pub #member: #ty, }
    });

    let writes = field_writes(errors);
    let reads = field_reads(errors);

    quote! {
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct ResponseError {
            pub result: crate::types::common::OperationResult,
            #(#members)*
        }

        impl ResponseError {
            pub fn from_tlvs(tlvs: &[TlvBuf]) -> Result<Self, Error> {
                let result = crate::result::decode_result(tlvs)?;
                let mut out = Self {
                    result,
                    ..Default::default()
                };

                {
                    let out = &mut out;
                    #(#reads)*
                }

                Ok(out)
            }

            pub fn to_tlvs(&self) -> Result<Vec<TlvBuf>, Error> {
                let mut result = TlvWriter::new();
                self.result.encode_to(&mut result)?;

                let mut tlvs = alloc::vec![result.into_tlv(crate::TLV_RESULT)];
                #(#writes)*
                Ok(tlvs)
            }
        }
    }
}

/// Whether a prerequisite is evaluated against the `Result` TLV.
fn result_guard(guard: &Guard) -> bool {
    guard
        .access
        .levels
        .first()
        .is_some_and(|level| level.rust_name == RESULT)
}

/// Whether a prerequisite only holds when the request failed.
fn failure_guard(guard: &Guard) -> bool {
    result_guard(guard) && (guard.op != "==" || guard.value != 0)
}

/// The fields a response decodes when the modem accepted the request.
///
/// The test on the `Result` TLV drops out: the arm already carries it, so a
/// field is optional here only when the database says so or when it depends on
/// another field.
fn data_fields(fields: &[RField]) -> Vec<RField> {
    let mut data: Vec<RField> = fields
        .iter()
        .filter(|field| !field.is_result && !field.guards.iter().any(failure_guard))
        .map(|field| {
            let mut field = field.clone();
            field.guards.retain(|guard| !result_guard(guard));
            field.optional = !field.mandatory || !field.guards.is_empty();
            field
        })
        .collect();

    // A prerequisite reads the field it depends on, which may have lost its
    // `Option` by moving into this arm.
    let optional: Vec<(String, bool)> = data
        .iter()
        .map(|field| (field.rust_name.clone(), field.optional))
        .collect();

    for field in &mut data {
        for guard in &mut field.guards {
            let Some(level) = guard.access.levels.first_mut() else {
                continue;
            };

            if let Some((_, optional)) = optional.iter().find(|(name, _)| *name == level.rust_name)
            {
                level.optional = *optional;
            }
        }
    }

    data
}

/// The fields a response only decodes when the modem rejected the request.
fn error_fields(fields: &[RField]) -> Vec<RField> {
    fields
        .iter()
        .filter(|field| field.guards.iter().any(failure_guard))
        .map(|field| {
            let mut field = field.clone();
            field.optional = true;
            field.guards = field.guards.iter().filter_map(error_guard).collect();
            field
        })
        .collect()
}

/// Drop the test of the error status from the prerequisite of a failure TLV:
/// the arm already carries it, and the error code reads as the member of the
/// `ResponseError`'s own result.
fn error_guard(guard: &Guard) -> Option<Guard> {
    assert!(
        result_guard(guard),
        "a TLV that only a failure carries is guarded by a success field"
    );

    (!(guard.op == "!=" && guard.value == 0)).then(|| guard.clone())
}

fn emit_to_tlvs(fields: &[RField]) -> TokenStream {
    if fields.is_empty() {
        return quote! {
            pub fn to_tlvs(&self) -> Result<Vec<TlvBuf>, Error> {
                Ok(Vec::new())
            }
        };
    }

    let writes = field_writes(fields);

    quote! {
        pub fn to_tlvs(&self) -> Result<Vec<TlvBuf>, Error> {
            let mut tlvs = Vec::new();
            #(#writes)*
            Ok(tlvs)
        }
    }
}

/// Emit one write of every field into the local `tlvs`.
fn field_writes(fields: &[RField]) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|field| {
            let rust_name = format_ident!("{}", field.rust_name);
            let id = Literal::u8_unsuffixed(field.tlv_id);

            if field.optional {
                let write = emit_write(&field.ty, quote! { value }, Ctx::Container);
                quote! {
                    if let Some(value) = &self.#rust_name {
                        let mut writer = TlvWriter::new();
                        #write
                        tlvs.push(writer.into_tlv(#id));
                    }
                }
            } else {
                let write = emit_write(&field.ty, quote! { &self.#rust_name }, Ctx::Container);
                quote! {
                    {
                        let mut writer = TlvWriter::new();
                        #write
                        tlvs.push(writer.into_tlv(#id));
                    }
                }
            }
        })
        .collect()
}

fn emit_from_tlvs(fields: &[RField]) -> TokenStream {
    if fields.is_empty() {
        return quote! {
            pub fn from_tlvs(_tlvs: &[TlvBuf]) -> Result<Self, Error> {
                Ok(Self::default())
            }
        };
    }

    let reads = field_reads(fields);

    quote! {
        pub fn from_tlvs(tlvs: &[TlvBuf]) -> Result<Self, Error> {
            let mut out = Self::default();
            #(#reads)*
            Ok(out)
        }
    }
}

/// Emit one read of every field out of `tlvs` into the local `out`.
fn field_reads(fields: &[RField]) -> Vec<TokenStream> {
    fields
        .iter()
        .map(|field| {
        let rust_name = format_ident!("{}", field.rust_name);
        let id = Literal::u8_unsuffixed(field.tlv_id);

        let read = if field.optional {
            let binding = emit_read_binding(&field.ty, quote! { value }, Ctx::Container);
            quote! {
                if let Some(tlv) = tlvs.iter().find(|tlv| tlv.id == #id) {
                    let mut reader = TlvReader::new(&tlv.value);
                    #binding
                    out.#rust_name = Some(value);
                }
            }
        } else {
            let read = emit_read(&field.ty, quote! { out.#rust_name }, Ctx::Container);
            quote! {
                let tlv = tlvs.iter().find(|tlv| tlv.id == #id).ok_or(Error::MissingTlv { id: #id })?;
                let mut reader = TlvReader::new(&tlv.value);
                #read
            }
        };

        if field.guards.is_empty() {
            read
        } else {
            let mut conditions = field
                .guards
                .iter()
                .map(|guard| guard_expr(quote! { out }, guard));

            let first = conditions.next().expect("guarded field without guards");
            let condition = conditions.fold(first, |acc, next| quote! { #acc && #next });

            quote! {
                if #condition {
                    #read
                }
            }
        }
        })
        .collect()
}

pub fn emit_composite(composite: &Composite) -> TokenStream {
    let name = format_ident!("{}", composite.name);

    match &composite.kind {
        CompositeKind::Struct(members) => {
            let extra = extra_derives(&composite.name);
            let fields = members.iter().map(|member| {
                let field = format_ident!("{}", member.rust_name);
                let ty = ty_tokens(&member.ty);
                quote! { pub #field: #ty, }
            });

            let writes = members.iter().map(|member| {
                let field = format_ident!("{}", member.rust_name);
                emit_write(&member.ty, quote! { &self.#field }, Ctx::Composite)
            });

            let reads = members.iter().map(|member| {
                let field = format_ident!("{}", member.rust_name);
                emit_read(&member.ty, quote! { out.#field }, Ctx::Composite)
            });

            let (writer, reader) = if members.is_empty() {
                (quote! { _writer }, quote! { _reader })
            } else {
                (quote! { writer }, quote! { reader })
            };

            let decode = if members.is_empty() {
                quote! { Ok(Self::default()) }
            } else {
                quote! {
                    let mut out = Self::default();
                    #(#reads)*
                    Ok(out)
                }
            };

            quote! {
                #[derive(Debug, Clone, PartialEq, Default #extra)]
                pub struct #name {
                    #(#fields)*
                }

                impl #name {
                    pub fn encode_to(&self, #writer: &mut TlvWriter) -> Result<(), Error> {
                        #(#writes)*
                        Ok(())
                    }

                    pub fn decode_from(#reader: &mut TlvReader<'_>) -> Result<Self, Error> {
                        #decode
                    }
                }
            }
        }
        CompositeKind::SeqArray { count_prefix, elem } => {
            let elem_ty = ty_tokens(elem);
            let count = emit_count_write(quote! { self.items }, *count_prefix);
            let write = emit_write(elem, quote! { item }, Ctx::Composite);
            let count_read = read_count_expr(*count_prefix);
            let element = emit_element_read(elem, Ctx::Composite);

            quote! {
                #[derive(Debug, Clone, PartialEq, Default)]
                pub struct #name {
                    pub sequence: u8,
                    pub items: Vec<#elem_ty>,
                }

                impl #name {
                    pub fn encode_to(&self, writer: &mut TlvWriter) -> Result<(), Error> {
                        #count
                        writer.write_u8(self.sequence);
                        for item in &self.items {
                            #write
                        }
                        Ok(())
                    }

                    pub fn decode_from(reader: &mut TlvReader<'_>) -> Result<Self, Error> {
                        let count = #count_read;
                        let sequence = reader.read_u8()?;
                        let mut items = Vec::with_capacity(count.min(256));
                        #element
                        Ok(Self { sequence, items })
                    }
                }
            }
        }
    }
}

/// Emit `writer` writes for a value of `ty`, where `expr` is a `&T`.
fn emit_write(ty: &RType, expr: TokenStream, ctx: Ctx) -> TokenStream {
    match ty {
        RType::U8 => quote! { writer.write_u8(*#expr); },
        RType::I8 => quote! { writer.write_i8(*#expr); },
        RType::U16(e) => {
            let e = endian(*e);
            quote! { writer.write_u16(*#expr, #e); }
        }
        RType::I16(e) => {
            let e = endian(*e);
            quote! { writer.write_i16(*#expr, #e); }
        }
        RType::U32(e) => {
            let e = endian(*e);
            quote! { writer.write_u32(*#expr, #e); }
        }
        RType::I32(e) => {
            let e = endian(*e);
            quote! { writer.write_i32(*#expr, #e); }
        }
        RType::U64(e) => {
            let e = endian(*e);
            quote! { writer.write_u64(*#expr, #e); }
        }
        RType::I64(e) => {
            let e = endian(*e);
            quote! { writer.write_i64(*#expr, #e); }
        }
        RType::F32(e) => {
            let e = endian(*e);
            quote! { writer.write_f32(*#expr, #e); }
        }
        RType::F64(e) => {
            let e = endian(*e);
            quote! { writer.write_f64(*#expr, #e); }
        }
        RType::Bool { signed: false } => quote! { writer.write_u8(u8::from(*#expr)); },
        RType::Bool { signed: true } => {
            quote! { writer.write_i8(if *#expr { 1 } else { 0 }); }
        }
        RType::SizedUint { bytes, e } => {
            let bytes = Literal::usize_unsuffixed(*bytes);
            let e = endian(*e);
            quote! { writer.write_uint(*#expr, #bytes, #e)?; }
        }
        RType::Str { fixed, .. } => match fixed {
            Some(size) => {
                let size = Literal::usize_unsuffixed(*size);
                quote! { writer.write_string(#expr, 0, Some(#size))?; }
            }
            None => {
                let prefix = Literal::u8_unsuffixed(str_prefix(ty));
                quote! { writer.write_string(#expr, #prefix, None)?; }
            }
        },
        RType::Composite(_) => {
            let writer = ctx.writer();
            quote! { (#expr).encode_to(#writer)?; }
        }
        RType::Array {
            elem,
            prefix,
            fixed,
        } => {
            let count = match fixed {
                Some(size) => {
                    let size = Literal::usize_unsuffixed(*size);
                    quote! { if (#expr).len() != #size { return Err(Error::InvalidLength); } }
                }
                None => emit_count_write(expr.clone(), prefix.expect("array without a prefix")),
            };

            let item = emit_write(elem, quote! { item }, ctx);

            quote! {
                #count
                for item in #expr {
                    #item
                }
            }
        }
    }
}

/// Emit a read into the place expression `target`.
fn emit_read(ty: &RType, target: TokenStream, ctx: Ctx) -> TokenStream {
    match ty {
        RType::Array {
            elem,
            prefix,
            fixed,
        } => emit_array_read(elem, *prefix, *fixed, target, false, ctx),
        _ => {
            let expr = read_expr(ty, ctx);
            quote! { #target = #expr; }
        }
    }
}

/// Emit a read that binds the value to a new local named `target`.
fn emit_read_binding(ty: &RType, target: TokenStream, ctx: Ctx) -> TokenStream {
    match ty {
        RType::Array {
            elem,
            prefix,
            fixed,
        } => emit_array_read(elem, *prefix, *fixed, target, true, ctx),
        _ => {
            let expr = read_expr(ty, ctx);
            quote! { let #target = #expr; }
        }
    }
}

fn emit_array_read(
    elem: &RType,
    prefix: Option<u8>,
    fixed: Option<usize>,
    target: TokenStream,
    bind: bool,
    ctx: Ctx,
) -> TokenStream {
    let count = match fixed {
        Some(size) => {
            let size = Literal::usize_unsuffixed(size);
            quote! { let count = #size; }
        }
        None => {
            let expr = read_count_expr(prefix.expect("array without a prefix"));
            quote! { let count = #expr; }
        }
    };

    let element = emit_element_read(elem, ctx);

    let assign = if bind {
        quote! { let #target = items; }
    } else {
        quote! { #target = items; }
    };

    quote! {
        #count
        let mut items = Vec::with_capacity(count.min(256));
        #element
        #assign
    }
}

/// Emit a loop pushing `count` elements into a local `items` vector.
fn emit_element_read(elem: &RType, ctx: Ctx) -> TokenStream {
    let push = match elem {
        RType::Array {
            elem: inner,
            prefix,
            fixed,
        } => {
            let read = emit_array_read(inner, *prefix, *fixed, quote! { item }, true, ctx);
            quote! {
                #read
                items.push(item);
            }
        }
        _ => {
            let expr = read_expr(elem, ctx);
            quote! { items.push(#expr); }
        }
    };

    quote! {
        for _ in 0..count {
            #push
        }
    }
}

/// Read expression for types that can be produced by a single call.
fn read_expr(ty: &RType, ctx: Ctx) -> TokenStream {
    match ty {
        RType::U8 => quote! { reader.read_u8()? },
        RType::I8 => quote! { reader.read_i8()? },
        RType::U16(e) => {
            let e = endian(*e);
            quote! { reader.read_u16(#e)? }
        }
        RType::I16(e) => {
            let e = endian(*e);
            quote! { reader.read_i16(#e)? }
        }
        RType::U32(e) => {
            let e = endian(*e);
            quote! { reader.read_u32(#e)? }
        }
        RType::I32(e) => {
            let e = endian(*e);
            quote! { reader.read_i32(#e)? }
        }
        RType::U64(e) => {
            let e = endian(*e);
            quote! { reader.read_u64(#e)? }
        }
        RType::I64(e) => {
            let e = endian(*e);
            quote! { reader.read_i64(#e)? }
        }
        RType::F32(e) => {
            let e = endian(*e);
            quote! { reader.read_f32(#e)? }
        }
        RType::F64(e) => {
            let e = endian(*e);
            quote! { reader.read_f64(#e)? }
        }
        RType::Bool { signed: false } => quote! { reader.read_u8()? != 0 },
        RType::Bool { signed: true } => quote! { reader.read_i8()? != 0 },
        RType::SizedUint { bytes, e } => {
            let bytes = Literal::usize_unsuffixed(*bytes);
            let e = endian(*e);
            quote! { reader.read_uint(#bytes, #e)? }
        }
        RType::Str { fixed, .. } => match fixed {
            Some(size) => {
                let size = Literal::usize_unsuffixed(*size);
                quote! { reader.read_fixed_string(#size)? }
            }
            None => {
                let prefix = Literal::u8_unsuffixed(str_prefix(ty));
                let max = Literal::usize_unsuffixed(str_max(ty));
                quote! { reader.read_string(#prefix, #max)? }
            }
        },
        RType::Composite(name) => {
            let name = composite_path(name);
            let reader = ctx.reader();
            quote! { #name::decode_from(#reader)? }
        }
        RType::Array { .. } => panic!("array has no read expression"),
    }
}

fn emit_count_write(expr: TokenStream, bytes: u8) -> TokenStream {
    match bytes {
        1 => quote! {
            writer.write_u8(u8::try_from((#expr).len()).map_err(|_| Error::TooLong)?);
        },
        2 => quote! {
            writer.write_u16(u16::try_from((#expr).len()).map_err(|_| Error::TooLong)?, Endian::Little);
        },
        4 => quote! {
            writer.write_u32(u32::try_from((#expr).len()).map_err(|_| Error::TooLong)?, Endian::Little);
        },
        _ => panic!("invalid array count width {bytes}"),
    }
}

fn read_count_expr(bytes: u8) -> TokenStream {
    match bytes {
        1 => quote! { reader.read_u8()? as usize },
        2 => quote! { reader.read_u16(Endian::Little)? as usize },
        4 => quote! { reader.read_u32(Endian::Little)? as usize },
        _ => panic!("invalid array count width {bytes}"),
    }
}

fn ty_tokens(ty: &RType) -> TokenStream {
    match ty {
        RType::U8 => quote! { u8 },
        RType::I8 => quote! { i8 },
        RType::U16(_) => quote! { u16 },
        RType::I16(_) => quote! { i16 },
        RType::U32(_) => quote! { u32 },
        RType::I32(_) => quote! { i32 },
        RType::U64(_) => quote! { u64 },
        RType::I64(_) => quote! { i64 },
        RType::F32(_) => quote! { f32 },
        RType::F64(_) => quote! { f64 },
        RType::Bool { .. } => quote! { bool },
        RType::SizedUint { .. } => quote! { u64 },
        RType::Str { .. } => quote! { String },
        RType::Composite(name) => composite_path(name),
        RType::Array { elem, .. } => {
            let elem = ty_tokens(elem);
            quote! { Vec<#elem> }
        }
    }
}

fn field_ty(field: &RField) -> TokenStream {
    let ty = ty_tokens(&field.ty);

    if field.optional {
        quote! { Option<#ty> }
    } else {
        ty
    }
}

/// Path of a composite type: the shared one for the types the database
/// repeats, the message's own module otherwise.
fn composite_path(composite: &str) -> TokenStream {
    let name = format_ident!("{composite}");

    if shared(composite) {
        quote! { crate::types::common::#name }
    } else {
        quote! { #name }
    }
}

fn endian(e: E) -> TokenStream {
    match e {
        E::Little => quote! { Endian::Little },
        E::Big => quote! { Endian::Big },
    }
}

fn str_prefix(ty: &RType) -> u8 {
    match ty {
        RType::Str { prefix, .. } => *prefix,
        _ => panic!("not a string"),
    }
}

fn str_max(ty: &RType) -> usize {
    match ty {
        RType::Str { max, .. } => *max,
        _ => panic!("not a string"),
    }
}

fn guard_expr(recv: TokenStream, guard: &Guard) -> TokenStream {
    let access = access_expr(recv, &guard.access.levels);

    let op = match guard.op {
        "==" => quote! { == },
        "!=" => quote! { != },
        other => panic!("unsupported guard operation {other}"),
    };

    let value = Literal::u64_unsuffixed(guard.value);

    quote! { #access #op #value }
}

fn access_expr(recv: TokenStream, levels: &[AccessLevel]) -> TokenStream {
    let (first, rest) = levels.split_first().expect("empty prerequisite path");
    let name = format_ident!("{}", first.rust_name);
    let this = quote! { #recv.#name };

    if rest.is_empty() {
        // An absent optional field reads as its default, like libqmi's
        // zero-initialized output structure.
        return if first.optional {
            quote! { #this.unwrap_or_default() }
        } else {
            this
        };
    }

    if first.optional {
        let inner = access_expr(quote! { v }, rest);
        quote! { #this.as_ref().map_or(0, |v| #inner) }
    } else {
        access_expr(this, rest)
    }
}
