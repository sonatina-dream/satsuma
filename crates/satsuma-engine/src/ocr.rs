//! On-device text recognition with the Vision framework.

use crate::{Error, Result};
use objc2::rc::autoreleasepool;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSDictionary, NSURL};
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};
use std::path::Path;

/// One recognised line; the box is normalised with a top-left origin.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub top: f64,
    pub height: f64,
}

pub fn recognize(path: &Path) -> Result<Vec<Line>> {
    autoreleasepool(|_| {
        let url = NSURL::from_file_path(path).ok_or_else(|| Error::msg("Invalid image path."))?;
        let handler = unsafe { VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new()) };
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|e| Error::msg(format!("Text recognition failed: {}", e.localizedDescription())))?;
        let mut lines = Vec::new();
        if let Some(results) = request.results() {
            for obs in results.iter() {
                let candidates = obs.topCandidates(1);
                let Some(best) = candidates.iter().next() else { continue };
                let b = unsafe { obs.boundingBox() };
                lines.push(Line {
                    text: best.string().to_string(),
                    x: b.origin.x,
                    top: 1.0 - (b.origin.y + b.size.height),
                    height: b.size.height,
                });
            }
        }
        Ok(lines)
    })
}

/// Orders lines top to bottom and joins them into paragraphs at larger vertical gaps.
pub fn paragraphs(mut lines: Vec<Line>) -> Vec<String> {
    lines.retain(|l| !l.text.trim().is_empty());
    lines.sort_by(|a, b| a.top.partial_cmp(&b.top).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
    let mut heights: Vec<f64> = lines.iter().map(|l| l.height).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let typical = heights.get(heights.len() / 2).copied().unwrap_or(0.02);
    let mut out: Vec<String> = Vec::new();
    let mut prev_bottom: Option<f64> = None;
    for line in lines {
        let gap = prev_bottom.map(|b| line.top - b);
        let new_paragraph = match gap {
            None => true,
            Some(g) => g > typical * 0.9,
        };
        let text = line.text.trim();
        if new_paragraph || out.is_empty() {
            out.push(text.to_string());
        } else {
            let last = out.last_mut().unwrap();
            if last.ends_with('-') {
                last.pop();
            } else {
                last.push(' ');
            }
            last.push_str(text);
        }
        prev_bottom = Some(line.top + line.height);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, top: f64) -> Line {
        Line { text: text.into(), x: 0.1, top, height: 0.02 }
    }

    #[test]
    fn groups_lines_into_paragraphs() {
        let lines = vec![
            line("second para", 0.30),
            line("Title", 0.05),
            line("first para starts and con-", 0.12),
            line("tinues here", 0.145),
        ];
        assert_eq!(paragraphs(lines), vec!["Title", "first para starts and continues here", "second para"]);
    }
}
