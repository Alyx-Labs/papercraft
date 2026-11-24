//! Regenerates desktop icon files from the canonical PaperCraft logo.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};

const PNG_SIZES: [u32; 9] = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
const WINDOWS_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const APPLE_SIZES: [(u32, &[u8; 4]); 7] = [
    (16, b"icp4"),
    (32, b"icp5"),
    (64, b"icp6"),
    (128, b"ic07"),
    (256, b"ic08"),
    (512, b"ic09"),
    (1024, b"ic10"),
];
const MAX_SOURCE_DIMENSION: u32 = 8192;
const MAX_SOURCE_PIXELS: u64 = 16_777_216;
const MASTER_SIZE: u32 = 1024;

pub fn run() -> Result<usize, String> {
    let branding_dir = crate::workspace_root()?
        .join("apps")
        .join("papercraft")
        .join("assets")
        .join("branding");
    let source_path = branding_dir.join("logo.png");
    let (source_width, source_height) = image_dimensions(&source_path)?;
    validate_dimensions(source_width, source_height)?;

    let source = ImageReader::open(&source_path)
        .map_err(|err| format!("could not open {}: {err}", source_path.display()))?
        .with_guessed_format()
        .map_err(|err| format!("could not identify {}: {err}", source_path.display()))?
        .decode()
        .map_err(|err| format!("could not decode {}: {err}", source_path.display()))?
        .into_rgba8();
    let master = fit_to_square(&source, MASTER_SIZE);

    let mut pngs = Vec::with_capacity(PNG_SIZES.len());
    for size in PNG_SIZES {
        let icon = resize_square(&master, size);
        let png = encode_png(icon)?;
        write_asset(&branding_dir.join(format!("papercraft-{size}.png")), &png)?;
        pngs.push((size, png));
    }

    let runtime_icon = resize_square(&master, 256).into_raw();
    write_asset(&branding_dir.join("papercraft-256.rgba"), &runtime_icon)?;
    write_asset(&branding_dir.join("papercraft.ico"), &encode_ico(&pngs)?)?;
    write_asset(&branding_dir.join("papercraft.icns"), &encode_icns(&pngs)?)?;

    println!(
        "source: {} ({} × {} px)",
        source_path.display(),
        source_width,
        source_height
    );
    Ok(pngs.len() + 3)
}

fn image_dimensions(path: &Path) -> Result<(u32, u32), String> {
    ImageReader::open(path)
        .map_err(|err| format!("could not open {}: {err}", path.display()))?
        .with_guessed_format()
        .map_err(|err| format!("could not identify {}: {err}", path.display()))?
        .into_dimensions()
        .map_err(|err| format!("could not read dimensions of {}: {err}", path.display()))
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    let pixel_count = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_SOURCE_DIMENSION
        || height > MAX_SOURCE_DIMENSION
        || pixel_count > MAX_SOURCE_PIXELS
    {
        return Err(format!(
            "logo dimensions {width} × {height} px exceed the supported limit of {MAX_SOURCE_DIMENSION} px per side and {MAX_SOURCE_PIXELS} total pixels"
        ));
    }
    Ok(())
}

fn fit_to_square(source: &RgbaImage, size: u32) -> RgbaImage {
    let mut square = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let scale = (f64::from(size) / f64::from(source.width()))
        .min(f64::from(size) / f64::from(source.height()));
    let width = (f64::from(source.width()) * scale)
        .round()
        .clamp(1.0, f64::from(size)) as u32;
    let height = (f64::from(source.height()) * scale)
        .round()
        .clamp(1.0, f64::from(size)) as u32;
    let fitted = imageops::resize(source, width, height, FilterType::Lanczos3);
    let x = i64::from((size - width) / 2);
    let y = i64::from((size - height) / 2);
    imageops::overlay(&mut square, &fitted, x, y);
    square
}

fn resize_square(source: &RgbaImage, size: u32) -> RgbaImage {
    imageops::resize(source, size, size, FilterType::Lanczos3)
}

fn encode_png(image: RgbaImage) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|err| format!("could not encode PNG icon: {err}"))?;
    Ok(output.into_inner())
}

fn encode_ico(pngs: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let entry_count = u16::try_from(WINDOWS_SIZES.len())
        .map_err(|_| "too many Windows icon sizes".to_string())?;
    let table_size = u32::from(entry_count)
        .checked_mul(16)
        .and_then(|size| size.checked_add(6))
        .ok_or_else(|| "Windows icon table size overflow".to_string())?;
    let mut output = Vec::new();
    output.extend_from_slice(&0_u16.to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&entry_count.to_le_bytes());

    let mut next_offset = table_size;
    for size in WINDOWS_SIZES {
        let png = png_for_size(pngs, size)?;
        let png_length = u32::try_from(png.len())
            .map_err(|_| format!("{size} px icon is too large for Windows ICO"))?;
        let dimension = if size == 256 {
            0
        } else {
            u8::try_from(size).map_err(|_| format!("invalid Windows icon size {size}"))?
        };
        output.push(dimension);
        output.push(dimension);
        output.push(0);
        output.push(0);
        output.extend_from_slice(&1_u16.to_le_bytes());
        output.extend_from_slice(&32_u16.to_le_bytes());
        output.extend_from_slice(&png_length.to_le_bytes());
        output.extend_from_slice(&next_offset.to_le_bytes());
        next_offset = next_offset
            .checked_add(png_length)
            .ok_or_else(|| "Windows icon file size overflow".to_string())?;
    }

    for size in WINDOWS_SIZES {
        output.extend_from_slice(png_for_size(pngs, size)?);
    }
    Ok(output)
}

fn encode_icns(pngs: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut chunks = Vec::new();
    for (size, chunk_type) in APPLE_SIZES {
        let png = png_for_size(pngs, size)?;
        let chunk_length = png
            .len()
            .checked_add(8)
            .and_then(|length| u32::try_from(length).ok())
            .ok_or_else(|| format!("{size} px icon is too large for Apple ICNS"))?;
        chunks.extend_from_slice(chunk_type);
        chunks.extend_from_slice(&chunk_length.to_be_bytes());
        chunks.extend_from_slice(png);
    }

    let file_length = chunks
        .len()
        .checked_add(8)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| "Apple ICNS file size overflow".to_string())?;
    let mut output = Vec::with_capacity(file_length as usize);
    output.extend_from_slice(b"icns");
    output.extend_from_slice(&file_length.to_be_bytes());
    output.extend_from_slice(&chunks);
    Ok(output)
}

fn png_for_size(pngs: &[(u32, Vec<u8>)], size: u32) -> Result<&[u8], String> {
    pngs.iter()
        .find(|(candidate, _)| *candidate == size)
        .map(|(_, png)| png.as_slice())
        .ok_or_else(|| format!("missing generated {size} px PNG"))
}

fn write_asset(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|err| format!("could not write {}: {err}", path.display()))
}
