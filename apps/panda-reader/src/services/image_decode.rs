//! Decode article images away from the UI thread and bound their memory cost.

pub(crate) fn decode_image_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| error.to_string())?;
    if u64::from(width) * u64::from(height) > 64_000_000 {
        return Err("Image exceeds the 64 megapixel decode limit".to_owned());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_small_image_and_rejects_oversized_dimensions() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let converted = decode_image_png(&png.into_inner()).unwrap();
        assert_eq!(image::load_from_memory(&converted).unwrap().width(), 2);

        // A BMP header can declare a huge image without allocating its pixels.
        let mut bmp = vec![0_u8; 54];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&(54_u32 + 9000 * 9000 * 3).to_le_bytes());
        bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&9000_i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&9000_i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bmp[34..38].copy_from_slice(&(9000_u32 * 9000 * 3).to_le_bytes());
        let error = decode_image_png(&bmp).unwrap_err();
        assert!(error.contains("64 megapixel"), "{error}");
    }
}
