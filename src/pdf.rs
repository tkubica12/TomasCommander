use crate::files::FileResult;

pub struct Page {
    pub index: u32,
    pub count: u32,
    pub png: Vec<u8>,
}

#[cfg(windows)]
struct Apartment;

#[cfg(windows)]
impl Apartment {
    // COM initialization is balanced on the calling worker thread, never the UI thread.
    fn enter() -> windows_core::Result<Self> {
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
        }?;
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::WinRT::RoUninitialize() };
    }
}

#[cfg(windows)]
struct PageClose(windows::Data::Pdf::PdfPage);

#[cfg(windows)]
impl Drop for PageClose {
    fn drop(&mut self) {
        if let Err(error) = self.0.Close() {
            eprintln!("Close native PDF page: {error}");
        }
    }
}

#[cfg(windows)]
fn load_native(
    bytes: &[u8],
    wait: &dyn Fn(&windows_future::IAsyncInfo) -> FileResult<()>,
) -> FileResult<windows::Data::Pdf::PdfDocument> {
    use windows::{
        Data::Pdf::PdfDocument,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    };
    use windows_core::Interface;

    let input = InMemoryRandomAccessStream::new()
        .map_err(|error| format!("Create PDF input stream: {error}"))?;
    let writer = DataWriter::CreateDataWriter(
        &input
            .GetOutputStreamAt(0)
            .map_err(|error| format!("Create PDF output interface: {error}"))?,
    )
    .map_err(|error| format!("Create PDF input writer: {error}"))?;
    writer
        .WriteBytes(bytes)
        .map_err(|error| format!("Buffer PDF input: {error}"))?;
    let store = writer
        .StoreAsync()
        .map_err(|error| format!("Store PDF input: {error}"))?;
    wait(
        &store
            .cast()
            .map_err(|error| format!("Inspect PDF store operation: {error}"))?,
    )?;
    if store
        .GetResults()
        .map_err(|error| format!("Complete PDF input: {error}"))? as usize
        != bytes.len()
    {
        return Err("PDF input stream write was incomplete.".into());
    }
    writer
        .DetachStream()
        .map_err(|error| format!("Detach PDF input writer: {error}"))?;
    input
        .Seek(0)
        .map_err(|error| format!("Rewind PDF input: {error}"))?;
    let loading = PdfDocument::LoadFromStreamAsync(&input)
        .map_err(|error| format!("Start native PDF loading: {error}"))?;
    wait(
        &loading
            .cast()
            .map_err(|error| format!("Inspect PDF loading: {error}"))?,
    )?;
    loading
        .GetResults()
        .map_err(|error| format!("Load native PDF: {error}"))
}

#[cfg(windows)]
pub fn render(bytes: &[u8], index: u32, cancelled: impl Fn() -> bool) -> FileResult<Page> {
    use std::time::{Duration, Instant};
    use windows::{
        Data::Pdf::PdfPageRenderOptions,
        Storage::Streams::{DataReader, InMemoryRandomAccessStream},
    };
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    if bytes.len() > 16 * 1_048_576 || cancelled() {
        return Err("PDF input exceeds 16 MiB or the request was cancelled.".into());
    }
    let _apartment =
        Apartment::enter().map_err(|error| format!("Initialize native PDF worker: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let wait = |operation: &IAsyncInfo| -> FileResult<()> {
        while operation
            .Status()
            .map_err(|error| format!("PDF operation status: {error}"))?
            == AsyncStatus::Started
        {
            if cancelled() || Instant::now() >= deadline {
                operation
                    .Cancel()
                    .map_err(|error| format!("Cancel native PDF operation: {error}"))?;
                return Err("PDF request cancelled or exceeded its 30-second deadline.".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if cancelled() || Instant::now() >= deadline {
            return Err("PDF request cancelled or exceeded its 30-second deadline.".into());
        }
        Ok(())
    };
    let document = load_native(bytes, &wait)?;
    let count = document
        .PageCount()
        .map_err(|error| format!("Read PDF page count: {error}"))?;
    if count == 0 || count > 200 || index >= count {
        return Err("PDF must contain 1-200 pages and the requested page must exist.".into());
    }
    let page = document
        .GetPage(index)
        .map_err(|error| format!("Read PDF page: {error}"))?;
    let _page_close = PageClose(page.clone());
    let size = page
        .Size()
        .map_err(|error| format!("Read PDF page dimensions: {error}"))?;
    if !size.Width.is_finite()
        || !size.Height.is_finite()
        || size.Width <= 0.0
        || size.Height <= 0.0
    {
        return Err("PDF has invalid page dimensions.".into());
    }
    let scale = (2048.0 / size.Width).min(2048.0 / size.Height).min(1.0);
    let options = PdfPageRenderOptions::new()
        .map_err(|error| format!("Create PDF render options: {error}"))?;
    options
        .SetDestinationWidth((size.Width * scale).round().max(1.0) as u32)
        .map_err(|error| format!("Set PDF render width: {error}"))?;
    options
        .SetDestinationHeight((size.Height * scale).round().max(1.0) as u32)
        .map_err(|error| format!("Set PDF render height: {error}"))?;
    let output = InMemoryRandomAccessStream::new()
        .map_err(|error| format!("Create PDF image stream: {error}"))?;
    let rendering = page
        .RenderWithOptionsToStreamAsync(&output, &options)
        .map_err(|error| format!("Start native PDF rendering: {error}"))?;
    wait(
        &rendering
            .cast()
            .map_err(|error| format!("Inspect PDF rendering: {error}"))?,
    )?;
    rendering
        .GetResults()
        .map_err(|error| format!("Render native PDF: {error}"))?;
    let length = output
        .Size()
        .map_err(|error| format!("Read PDF image size: {error}"))?;
    if length == 0 || length > 16 * 1_048_576 {
        return Err("PDF rendered image exceeds the 16 MiB limit or is empty.".into());
    }
    let reader = DataReader::CreateDataReader(
        &output
            .GetInputStreamAt(0)
            .map_err(|error| format!("Read PDF image stream: {error}"))?,
    )
    .map_err(|error| format!("Create PDF image reader: {error}"))?;
    let reading = reader
        .LoadAsync(length as u32)
        .map_err(|error| format!("Load PDF image bytes: {error}"))?;
    wait(
        &reading
            .cast()
            .map_err(|error| format!("Inspect PDF image reading: {error}"))?,
    )?;
    if reading
        .GetResults()
        .map_err(|error| format!("Complete PDF image reading: {error}"))? as u64
        != length
    {
        return Err("PDF rendered image was incompletely read.".into());
    }
    let mut png = vec![0; length as usize];
    reader
        .ReadBytes(&mut png)
        .map_err(|error| format!("Read PDF PNG: {error}"))?;
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return Err("Native PDF renderer returned an unexpected image format.".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().expect("bounded PNG width"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("bounded PNG height"));
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(
            "Native PDF bitmap exceeds the 4096-pixel edge limit at this display DPI.".into(),
        );
    }
    Ok(Page { index, count, png })
}

#[cfg(not(windows))]
pub fn render(_bytes: &[u8], _index: u32, _cancelled: impl Fn() -> bool) -> FileResult<Page> {
    Err("Native Windows PDF rendering requires Windows.".into())
}

/// Windows.Data.Pdf rendering plus Windows.Media.Ocr for scanned PDF pages.
/// It runs in the caller's (contained worker) thread; nothing leaves the process.
#[cfg(windows)]
pub(crate) struct NativeOcr {
    bytes: std::sync::Arc<[u8]>,
    session: Option<OcrSession>,
}

#[cfg(windows)]
struct OcrSession {
    document: windows::Data::Pdf::PdfDocument,
    engine: windows::Media::Ocr::OcrEngine,
    // Declared last so WinRT objects are released before the apartment.
    _apartment: Apartment,
}

#[cfg(windows)]
impl NativeOcr {
    pub(crate) fn new(bytes: std::sync::Arc<[u8]>) -> Self {
        Self {
            bytes,
            session: None,
        }
    }
}

#[cfg(windows)]
fn ocr_wait(
    operation: &windows_future::IAsyncInfo,
    deadline: std::time::Instant,
    stopped: &dyn Fn() -> bool,
) -> FileResult<()> {
    use windows_future::AsyncStatus;
    while operation
        .Status()
        .map_err(|error| format!("OCR operation status: {error}"))?
        == AsyncStatus::Started
    {
        if stopped() || std::time::Instant::now() >= deadline {
            let _ = operation.Cancel();
            return Err("OCR page time budget exceeded or cancelled".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
}

#[cfg(windows)]
impl crate::pdf_text::PageOcr for NativeOcr {
    fn prepare(
        &mut self,
        deadline: std::time::Instant,
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), crate::pdf_text::OcrError> {
        use crate::pdf_text::OcrError;
        use windows::Media::Ocr::OcrEngine;
        if self.session.is_some() {
            return Ok(());
        }
        let apartment = Apartment::enter()
            .map_err(|error| OcrError::Unavailable(format!("Windows OCR unavailable: {error}")))?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
            OcrError::Unavailable(
                "Windows OCR is unavailable: no OCR language is installed for the user profile languages".into(),
            )
        })?;
        let document = load_native(&self.bytes, &|operation| {
            ocr_wait(operation, deadline, stopped)
        })
        .map_err(|error| {
            OcrError::Unavailable(format!("Windows could not open this PDF for OCR: {error}"))
        })?;
        self.session = Some(OcrSession {
            document,
            engine,
            _apartment: apartment,
        });
        Ok(())
    }

    fn recognize(
        &mut self,
        page_index: u32,
        deadline: std::time::Instant,
        stopped: &dyn Fn() -> bool,
    ) -> Result<String, crate::pdf_text::OcrError> {
        use crate::pdf_text::OcrError;
        use windows::{
            Data::Pdf::PdfPageRenderOptions,
            Graphics::Imaging::{
                BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform,
                ColorManagementMode, ExifOrientationMode,
            },
            Media::Ocr::OcrEngine,
            Storage::Streams::InMemoryRandomAccessStream,
        };
        use windows_core::Interface;
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| OcrError::Unavailable("Windows OCR was not prepared".into()))?;
        let failed = |stage: &str, error: windows_core::Error| {
            OcrError::Failed(format!("{stage}: {}", error.message()))
        };
        let wait = |operation: &windows_future::IAsyncInfo| {
            ocr_wait(operation, deadline, stopped).map_err(OcrError::Failed)
        };
        let page = session
            .document
            .GetPage(page_index)
            .map_err(|error| failed("read page", error))?;
        let _page_close = PageClose(page.clone());
        let size = page.Size().map_err(|error| failed("page size", error))?;
        if !size.Width.is_finite()
            || !size.Height.is_finite()
            || size.Width <= 0.0
            || size.Height <= 0.0
        {
            return Err(OcrError::Failed("invalid page dimensions".into()));
        }
        let maximum = OcrEngine::MaxImageDimension()
            .unwrap_or(2600)
            .clamp(512, 4096) as f32;
        // About 200 DPI for A4 keeps body text legible within the OCR size limit.
        let scale = maximum.min(2400.0) / size.Width.max(size.Height);
        let options = PdfPageRenderOptions::new().map_err(|error| failed("options", error))?;
        options
            .SetDestinationWidth((size.Width * scale).round().max(1.0) as u32)
            .map_err(|error| failed("render width", error))?;
        options
            .SetDestinationHeight((size.Height * scale).round().max(1.0) as u32)
            .map_err(|error| failed("render height", error))?;
        let output = InMemoryRandomAccessStream::new().map_err(|error| failed("stream", error))?;
        let rendering = page
            .RenderWithOptionsToStreamAsync(&output, &options)
            .map_err(|error| failed("render", error))?;
        wait(&rendering.cast().map_err(|error| failed("render", error))?)?;
        rendering
            .GetResults()
            .map_err(|error| failed("render", error))?;
        output.Seek(0).map_err(|error| failed("rewind", error))?;
        let decoding =
            BitmapDecoder::CreateAsync(&output).map_err(|error| failed("decode", error))?;
        wait(&decoding.cast().map_err(|error| failed("decode", error))?)?;
        let decoder = decoding
            .GetResults()
            .map_err(|error| failed("decode", error))?;
        let width = decoder
            .PixelWidth()
            .map_err(|error| failed("bitmap size", error))?;
        let height = decoder
            .PixelHeight()
            .map_err(|error| failed("bitmap size", error))?;
        if width == 0 || height == 0 {
            return Err(OcrError::Failed("empty page bitmap".into()));
        }
        let fit = (maximum / width.max(height) as f32).min(1.0);
        let transform = BitmapTransform::new().map_err(|error| failed("transform", error))?;
        transform
            .SetScaledWidth(((width as f32 * fit).round() as u32).max(1))
            .map_err(|error| failed("transform", error))?;
        transform
            .SetScaledHeight(((height as f32 * fit).round() as u32).max(1))
            .map_err(|error| failed("transform", error))?;
        let converting = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .map_err(|error| failed("bitmap", error))?;
        wait(&converting.cast().map_err(|error| failed("bitmap", error))?)?;
        let bitmap = converting
            .GetResults()
            .map_err(|error| failed("bitmap", error))?;
        let recognizing = session
            .engine
            .RecognizeAsync(&bitmap)
            .map_err(|error| failed("recognize", error))?;
        wait(
            &recognizing
                .cast()
                .map_err(|error| failed("recognize", error))?,
        )?;
        let result = recognizing
            .GetResults()
            .map_err(|error| failed("recognize", error))?;
        let lines = result.Lines().map_err(|error| failed("lines", error))?;
        let mut text = String::new();
        for index in 0..lines.Size().map_err(|error| failed("lines", error))? {
            let line = lines.GetAt(index).map_err(|error| failed("line", error))?;
            text.push_str(
                &line
                    .Text()
                    .map_err(|error| failed("line", error))?
                    .to_string_lossy(),
            );
            text.push('\n');
        }
        Ok(text)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let black = "0 0 0 rg\n10 10 20 30 re f\n";
        let red = "1 0 0 rg\n10 10 20 30 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 120] /Resources << >> /Contents 4 0 R >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{black}endstream", black.len()),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 120] /Resources << >> /Contents 6 0 R >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{red}endstream", red.len()),
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }

    #[test]
    fn native_pdf_renders_exact_page_pixels_and_refuses_invalid_cancelled_input() {
        let bytes = fixture();
        for (index, color) in [(0, [0, 0, 0]), (1, [255, 0, 0])] {
            let page = render(&bytes, index, || false).unwrap();
            assert_eq!((page.index, page.count), (index, 2));
            let image = image::load_from_memory(&page.png).unwrap().to_rgb8();
            // WinRT destination dimensions are DIPs, not physical PNG pixels.
            assert!(image.width() > 0 && image.width() <= 4096);
            assert!(image.height() > 0 && image.height() <= 4096);
            assert!((image.height() as i64 * 5 - image.width() as i64 * 6).abs() <= 6);
            assert_eq!(
                image
                    .get_pixel(image.width() * 20 / 100, image.height() * 95 / 120)
                    .0,
                color
            );
            assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255]);
        }
        assert!(render(&bytes, 2, || false).is_err());
        assert!(render(b"not a PDF", 0, || false).is_err());
        assert!(render(&bytes, 0, || true).is_err());
    }
}
