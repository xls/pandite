//! Facts an encoded image file states about itself: container, codec,
//! channels, color and camera settings.
//!
//! Decoders read them from the file headers
//! ([`crate::DecoderProvider::facts`]) without decoding pixels, so an
//! information display costs a header parse. Every field is optional: a
//! fact the format does not state, or the decoder does not read, is `None`.

/// Camera settings recorded in the file (EXIF). Each field is `None` when
/// the file does not record it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraFacts {
    /// Camera maker, for example `"SONY"`.
    pub make: Option<String>,
    /// Camera model, for example `"ILCE-7M4"`.
    pub model: Option<String>,
    /// Lens model.
    pub lens: Option<String>,
    /// Exposure time in seconds as a fraction: numerator and denominator,
    /// for example `(1, 250)`. The denominator is never zero.
    pub exposure: Option<(u32, u32)>,
    /// Aperture as an f-number, for example `2.8`.
    pub f_number: Option<f32>,
    /// ISO speed.
    pub iso: Option<u32>,
    /// Focal length in millimeters.
    pub focal_length: Option<f32>,
    /// When the picture was taken, as `"YYYY-MM-DD HH:MM:SS"` in the
    /// camera's own clock.
    pub captured: Option<String>,
}

impl CameraFacts {
    /// True when no field is known.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Facts about an encoded image file, read from its headers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageFacts {
    /// The container or file format, for example `"PNG"`, `"JPEG"`,
    /// `"Sony ARW"` or `"HEIF"`.
    pub container: Option<String>,
    /// How the pixels are compressed, for example `"Deflate"`,
    /// `"Baseline DCT, Huffman"`, `"VP8L lossless"` or `"HEVC"`.
    pub compression: Option<String>,
    /// A four-character code that names the codec or brand, where the
    /// format has one: `"hvc1"`, `"av01"`, `"DXT1"`, `"VP8L"`.
    pub fourcc: Option<String>,
    /// The stored channels, for example `"RGB"`, `"RGBA"`, `"Gray"`,
    /// `"CMYK"`, `"Indexed"` or `"Bayer RGGB"`.
    pub channels: Option<String>,
    /// Number of stored channels.
    pub channel_count: Option<u8>,
    /// Bits of one channel sample (the palette index size for indexed
    /// images).
    pub bits_per_channel: Option<u8>,
    /// Bits of one pixel as stored, when the format states it and it is
    /// not simply channels times bits per channel (packed layouts, block
    /// compressed textures).
    pub bits_per_pixel: Option<u16>,
    /// Chroma subsampling, for example `"4:2:0"`.
    pub subsampling: Option<String>,
    /// The color space the file declares, for example `"sRGB"` or
    /// `"Display P3"`.
    pub color_space: Option<String>,
    /// The description of the embedded ICC profile.
    pub icc_profile: Option<String>,
    /// Camera settings.
    pub camera: CameraFacts,
}

impl ImageFacts {
    /// Facts that name only the container.
    pub fn named(container: impl Into<String>) -> Self {
        Self {
            container: Some(container.into()),
            ..Self::default()
        }
    }

    /// Bits one pixel takes before compression: [`ImageFacts::bits_per_pixel`]
    /// when the file states it, otherwise channels times bits per channel.
    pub fn stored_bits_per_pixel(&self) -> Option<u32> {
        if let Some(bits) = self.bits_per_pixel.filter(|&b| b > 0) {
            return Some(u32::from(bits));
        }
        let channels = u32::from(self.channel_count?);
        let bits = u32::from(self.bits_per_channel?);
        (channels > 0 && bits > 0).then_some(channels * bits)
    }

    /// Set the channel name, count and bits per channel at once.
    pub fn set_channels(&mut self, name: impl Into<String>, count: u8, bits: u8) {
        self.channels = Some(name.into());
        self.channel_count = Some(count);
        self.bits_per_channel = Some(bits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_bits_follow_channels_unless_stated() {
        let mut f = ImageFacts::named("PNG");
        assert_eq!(f.container.as_deref(), Some("PNG"));
        assert_eq!(f.stored_bits_per_pixel(), None);
        f.set_channels("RGBA", 4, 16);
        assert_eq!(f.stored_bits_per_pixel(), Some(64));
        f.bits_per_pixel = Some(4);
        assert_eq!(f.stored_bits_per_pixel(), Some(4));
        assert!(f.camera.is_empty());
        f.camera.iso = Some(100);
        assert!(!f.camera.is_empty());
    }
}
