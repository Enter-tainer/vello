//! Generation of GLSL struct definitions and accessor functions.

use std::fmt::Write;
use std::ops::Deref;

use crate::layout::{LayoutModule, LayoutType, LayoutTypeDef};
use crate::parse::{GpuScalar, GpuType};

pub fn gen_glsl(module: &LayoutModule) -> String {
    let mut r = String::new();
    writeln!(&mut r, "// Code auto-generated by piet-gpu-derive\n").unwrap();
    // Note: GLSL needs definitions before uses. We could do a topological sort here,
    // but easiest for now to just require that in spec.
    for name in &module.def_names {
        gen_refdef(&mut r, &name);
    }
    for name in &module.def_names {
        match module.defs.get(name).unwrap() {
            (size, LayoutTypeDef::Struct(fields)) => {
                gen_struct_def(&mut r, name, fields);
                gen_item_def(&mut r, name, size.size);
            }
            (size, LayoutTypeDef::Enum(en)) => {
                gen_enum_def(&mut r, name, en);
                gen_item_def(&mut r, name, size.size);
            }
        }
    }
    for name in &module.def_names {
        let def = module.defs.get(name).unwrap();
        match def {
            (_size, LayoutTypeDef::Struct(fields)) => {
                gen_struct_read(&mut r, &module.name, &name, fields);
                if module.gpu_write {
                    gen_struct_write(&mut r, &module.name, &name, fields);
                }
            }
            (_size, LayoutTypeDef::Enum(en)) => {
                gen_enum_read(&mut r, &module.name, &name, en);
                if module.gpu_write {
                    gen_enum_write(&mut r, &module.name, &name, en);
                }
            }
        }
    }
    r
}

fn gen_refdef(r: &mut String, name: &str) {
    writeln!(r, "struct {}Ref {{", name).unwrap();
    writeln!(r, "    uint offset;").unwrap();
    writeln!(r, "}};\n").unwrap();
}

fn gen_struct_def(r: &mut String, name: &str, fields: &[(String, usize, LayoutType)]) {
    writeln!(r, "struct {} {{", name).unwrap();
    for (name, _offset, ty) in fields {
        writeln!(r, "    {} {};", glsl_type(&ty.ty), name).unwrap();
    }
    writeln!(r, "}};\n").unwrap();
}

fn gen_enum_def(r: &mut String, name: &str, variants: &[(String, Vec<(usize, LayoutType)>)]) {
    for (i, (var_name, _payload)) in variants.iter().enumerate() {
        writeln!(r, "#define {}_{} {}", name, var_name, i).unwrap();
    }
}

fn gen_item_def(r: &mut String, name: &str, size: usize) {
    writeln!(r, "#define {}_size {}\n", name, size).unwrap();
    writeln!(
        r,
        "{}Ref {}_index({}Ref ref, uint index) {{",
        name, name, name
    )
    .unwrap();
    writeln!(
        r,
        "    return {}Ref(ref.offset + index * {}_size);",
        name, name
    )
    .unwrap();
    writeln!(r, "}}\n").unwrap();
}

fn gen_struct_read(
    r: &mut String,
    bufname: &str,
    name: &str,
    fields: &[(String, usize, LayoutType)],
) {
    writeln!(r, "{} {}_read({}Ref ref) {{", name, name, name).unwrap();
    writeln!(r, "    uint ix = ref.offset >> 2;").unwrap();
    let coverage = crate::layout::struct_coverage(fields, false);
    for (i, fields) in coverage.iter().enumerate() {
        if !fields.is_empty() {
            writeln!(r, "    uint raw{} = {}[ix + {}];", i, bufname, i).unwrap();
        }
    }
    writeln!(r, "    {} s;", name).unwrap();
    for (name, offset, ty) in fields {
        writeln!(r, "    s.{} = {};", name, gen_extract(*offset, &ty.ty)).unwrap();
    }
    writeln!(r, "    return s;").unwrap();
    writeln!(r, "}}\n").unwrap();
}

fn gen_enum_read(
    r: &mut String,
    bufname: &str,
    name: &str,
    variants: &[(String, Vec<(usize, LayoutType)>)],
) {
    writeln!(r, "uint {}_tag({}Ref ref) {{", name, name).unwrap();
    writeln!(r, "    return {}[ref.offset >> 2];", bufname).unwrap();
    writeln!(r, "}}\n").unwrap();
    for (var_name, payload) in variants {
        if payload.len() == 1 {
            if let GpuType::InlineStruct(structname) = &payload[0].1.ty {
                writeln!(
                    r,
                    "{} {}_{}_read({}Ref ref) {{",
                    structname, name, var_name, name
                )
                .unwrap();
                writeln!(
                    r,
                    "    return {}_read({}Ref(ref.offset + {}));",
                    structname, structname, payload[0].0
                )
                .unwrap();
                writeln!(r, "}}\n").unwrap();
            }
        }
        // TODO: support for variants that aren't one struct.
    }
}

fn gen_extract(offset: usize, ty: &GpuType) -> String {
    match ty {
        GpuType::Scalar(scalar) => gen_extract_scalar(offset, scalar),
        GpuType::Vector(scalar, size) => {
            let mut r = glsl_type(ty);
            r.push_str("(");
            for i in 0..*size {
                if i != 0 {
                    r.push_str(", ");
                }
                let el_offset = offset + i * scalar.size();
                r.push_str(&gen_extract_scalar(el_offset, scalar));
            }
            r.push_str(")");
            r
        }
        GpuType::InlineStruct(name) => format!(
            "{}_read({}Ref({}))",
            name,
            name,
            simplified_add("ref.offset", offset)
        ),
        GpuType::Ref(inner) => {
            if let GpuType::InlineStruct(name) = inner.deref() {
                format!(
                    "{}Ref({})",
                    name,
                    gen_extract_scalar(offset, &GpuScalar::U32)
                )
            } else {
                panic!("only know how to deal with Ref of struct")
            }
        }
    }
}

fn gen_extract_scalar(offset: usize, ty: &GpuScalar) -> String {
    match ty {
        GpuScalar::F32 => format!("uintBitsToFloat(raw{})", offset / 4),
        GpuScalar::U8 | GpuScalar::U16 | GpuScalar::U32 => extract_ubits(offset, ty.size()),
        GpuScalar::I8 | GpuScalar::I16 | GpuScalar::I32 => extract_ibits(offset, ty.size()),
    }
}

fn extract_ubits(offset: usize, nbytes: usize) -> String {
    if nbytes == 4 {
        return format!("raw{}", offset / 4);
    }
    let mask = (1 << (nbytes * 8)) - 1;
    if offset % 4 == 0 {
        format!("raw{} & 0x{:x}", offset / 4, mask)
    } else if offset % 4 + nbytes == 4 {
        format!("raw{} >> {}", offset / 4, (offset % 4) * 8)
    } else {
        format!("(raw{} >> {}) & 0x{:x}", offset / 4, (offset % 4) * 8, mask)
    }
}

fn extract_ibits(offset: usize, nbytes: usize) -> String {
    if nbytes == 4 {
        return format!("int(raw{})", offset / 4);
    }
    if offset % 4 + nbytes == 4 {
        format!("int(raw{}) >> {}", offset / 4, (offset % 4) * 8)
    } else {
        format!(
            "int(raw{} << {}) >> {}",
            offset / 4,
            ((4 - nbytes) - offset % 4) * 8,
            (4 - nbytes) * 8
        )
    }
}

// Writing

fn gen_struct_write(
    r: &mut String,
    bufname: &str,
    name: &str,
    fields: &[(String, usize, LayoutType)],
) {
    writeln!(r, "void {}_write({}Ref ref, {} s) {{", name, name, name).unwrap();
    let coverage = crate::layout::struct_coverage(fields, true);
    for (i, field_ixs) in coverage.iter().enumerate() {
        let mut pieces = Vec::new();
        for field_ix in field_ixs {
            let (name, offset, ty) = &fields[*field_ix];
            match &ty.ty {
                GpuType::Scalar(scalar) => {
                    let inner = format!("s.{}", name);
                    pieces.push(gen_pack_bits_scalar(scalar, *offset, &inner));
                }
                GpuType::Vector(scalar, len) => {
                    let size = scalar.size();
                    let ix_lo = (i * 4 - offset) / size;
                    let ix_hi = ((4 + i * 4 - offset) / size).min(*len);
                    for ix in ix_lo..ix_hi {
                        let scalar_offset = offset + ix * size;
                        let inner = format!("s.{}.{}", name, &"xyzw"[ix..ix + 1]);
                        pieces.push(gen_pack_bits_scalar(scalar, scalar_offset, &inner));
                    }
                }
                GpuType::InlineStruct(structname) => {
                    writeln!(
                        r,
                        "    {}_write({}Ref({}), s.{});",
                        structname,
                        structname,
                        simplified_add("ref.offset", *offset),
                        name
                    )
                    .unwrap();
                }
                GpuType::Ref(_) => pieces.push(format!("s.{}.offset", name)),
            }
        }
        if !pieces.is_empty() {
            write!(r, "    {}[{}] = ", bufname, i).unwrap();
            for (j, piece) in pieces.iter().enumerate() {
                if j != 0 {
                    write!(r, " | ").unwrap();
                }
                write!(r, "{}", piece).unwrap();
            }
            writeln!(r, ";").unwrap();
        }
    }
    writeln!(r, "}}\n").unwrap();
}

fn gen_pack_bits_scalar(ty: &GpuScalar, offset: usize, inner: &str) -> String {
    let shift = (offset % 4) * 8;
    let bits = match ty {
        GpuScalar::F32 => format!("floatBitsToUint({})", inner),
        // Note: this doesn't mask small unsigned int types; the caller is
        // responsible for making sure they don't overflow.
        GpuScalar::U8 | GpuScalar::U16 | GpuScalar::U32 => inner.into(),
        GpuScalar::I8 => {
            if shift == 24 {
                format!("uint({})", inner)
            } else {
                format!("(uint({}) & 0xff)", inner)
            }
        }
        GpuScalar::I16 => {
            if shift == 16 {
                format!("uint({})", inner)
            } else {
                format!("(uint({}) & 0xffff)", inner)
            }
        }
        GpuScalar::I32 => format!("uint({})", inner),
    };
    if shift == 0 {
        bits
    } else {
        format!("({} << {})", bits, shift)
    }
}

fn gen_enum_write(
    r: &mut String,
    bufname: &str,
    name: &str,
    variants: &[(String, Vec<(usize, LayoutType)>)],
) {
    for (var_name, payload) in variants {
        if payload.is_empty() {
            writeln!(r, "void {}_{}_write({}Ref ref) {{", name, var_name, name).unwrap();
            writeln!(
                r,
                "    {}[ref.offset >> 2] = {}_{};",
                bufname, name, var_name
            )
            .unwrap();
            writeln!(r, "}}\n").unwrap();
        } else if payload.len() == 1 {
            if let GpuType::InlineStruct(structname) = &payload[0].1.ty {
                writeln!(
                    r,
                    "void {}_{}_write({}Ref ref, {} s) {{",
                    name, var_name, name, structname
                )
                .unwrap();
                writeln!(
                    r,
                    "    {}[ref.offset >> 2] = {}_{};",
                    bufname, name, var_name
                )
                .unwrap();
                writeln!(
                    r,
                    "    {}_write({}Ref(ref.offset + {}), s);",
                    structname, structname, payload[0].0
                )
                .unwrap();
                writeln!(r, "}}\n").unwrap();
            }
        }
        // TODO: support for variants that aren't one struct.
    }
}

// Utility functions

fn glsl_type(ty: &GpuType) -> String {
    match ty {
        GpuType::Scalar(scalar) => glsl_scalar(scalar).into(),
        GpuType::Vector(scalar, size) => {
            if *size == 1 {
                glsl_scalar(scalar).into()
            } else {
                format!("{}{}", glsl_vecname(scalar), size)
            }
        }
        GpuType::InlineStruct(name) => name.clone(),
        GpuType::Ref(inner) => {
            if let GpuType::InlineStruct(name) = inner.deref() {
                format!("{}Ref", name)
            } else {
                panic!("only know how to deal with Ref of struct")
            }
        }
    }
}

// GLSL type that can contain the scalar value.
fn glsl_scalar(s: &GpuScalar) -> &'static str {
    match s {
        GpuScalar::F32 => "float",
        GpuScalar::I8 | GpuScalar::I16 | GpuScalar::I32 => "int",
        GpuScalar::U8 | GpuScalar::U16 | GpuScalar::U32 => "uint",
    }
}

fn glsl_vecname(s: &GpuScalar) -> &'static str {
    match s {
        GpuScalar::F32 => "vec",
        GpuScalar::I8 | GpuScalar::I16 | GpuScalar::I32 => "ivec",
        GpuScalar::U8 | GpuScalar::U16 | GpuScalar::U32 => "uvec",
    }
}

/// If `c = 0`, return `"var_name"`, else `"var_name + c"`
fn simplified_add(var_name: &str, c: usize) -> String {
    if c == 0 {
        String::from(var_name)
    } else {
        format!("{} + {}", var_name, c)
    }
}
