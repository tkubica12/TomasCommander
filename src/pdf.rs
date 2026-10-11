use crate::files::FileResult;

pub struct Page {
    pub index: u32,
    pub count: u32,
    pub png: Vec<u8>,
}

#[cfg(windows)]
pub fn render(bytes: &[u8], index: u32, cancelled: impl Fn() -> bool) -> FileResult<Page> {
    use std::time::{Duration, Instant};
    use windows::{
        Data::Pdf::{PdfDocument, PdfPageRenderOptions},
        Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    };
    use windows_core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    if bytes.len() > 16 * 1_048_576 || cancelled() {
        return Err("PDF input exceeds 16 MiB or the request was cancelled.".into());
    }
    // COM initialization is balanced on this worker thread, never the UI thread.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        .map_err(|error| format!("Initialize native PDF worker: {error}"))?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    let _apartment = Apartment;
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
    let document = loading
        .GetResults()
        .map_err(|error| format!("Load native PDF: {error}"))?;
    let count = document
        .PageCount()
        .map_err(|error| format!("Read PDF page count: {error}"))?;
    if count == 0 || count > 200 || index >= count {
        return Err("PDF must contain 1-200 pages and the requested page must exist.".into());
    }
    let page = document
        .GetPage(index)
        .map_err(|error| format!("Read PDF page: {error}"))?;
    struct PageClose(windows::Data::Pdf::PdfPage);
    impl Drop for PageClose {
        fn drop(&mut self) {
            if let Err(error) = self.0.Close() {
                eprintln!("Close native PDF page: {error}");
            }
        }
    }
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
