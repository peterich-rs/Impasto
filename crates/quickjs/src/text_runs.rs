//! Length-prefixed text runs on the style text channel.
//!
//! ```text
//! <count>\n
//! <utf8ByteLen> <utf8><weight> <flags> <familyByteLen> <family><r>,<g>,<b>,<a> <gradTag>\n
//! ```
//!
//! `gradTag` 0 ends the run. `gradTag` 1 continues with a gradient in the same
//! unit square as [`protocol::Gradient`].

use protocol::{ColorSpace, ColorStop, Gradient, TextSpan};

struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn done(&self) -> bool {
        self.at >= self.text.len()
    }

    fn eat(&mut self, bytes: usize) -> Result<&'a str, uicore::Error> {
        let end = self.at.checked_add(bytes).ok_or(uicore::Error::BadNumber)?;
        if end > self.text.len() || !self.text.is_char_boundary(end) {
            return Err(uicore::Error::BadNumber);
        }
        let slice = &self.text[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn expect(&mut self, marker: char) -> Result<(), uicore::Error> {
        let next = self.rest().chars().next().ok_or(uicore::Error::BadNumber)?;
        if next != marker {
            return Err(uicore::Error::BadNumber);
        }
        self.at += marker.len_utf8();
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, uicore::Error> {
        let rest = self.rest();
        let len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return Err(uicore::Error::BadNumber);
        }
        let digits = &rest[..len];
        self.at += len;
        digits.parse().map_err(|_| uicore::Error::BadNumber)
    }

    fn f32(&mut self) -> Result<f32, uicore::Error> {
        let bytes = self.rest().as_bytes();
        let mut index = 0;
        if bytes.first() == Some(&b'-') {
            index = 1;
        }
        let digits = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == digits {
            return Err(uicore::Error::BadNumber);
        }
        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            let fraction = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index == fraction {
                return Err(uicore::Error::BadNumber);
            }
        }
        let token = &self.rest()[..index];
        self.at += index;
        let value: f32 = token.parse().map_err(|_| uicore::Error::BadNumber)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(uicore::Error::BadNumber)
        }
    }

    fn byte_field(&mut self) -> Result<&'a str, uicore::Error> {
        let len = usize::try_from(self.u32()?).map_err(|_| uicore::Error::BadNumber)?;
        self.expect(' ')?;
        self.eat(len)
    }
}

fn weight(value: u32) -> Result<u16, uicore::Error> {
    if matches!(value, 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900) {
        Ok(u16::try_from(value).unwrap_or(400))
    } else {
        Err(uicore::Error::BadNumber)
    }
}

fn byte(value: u32) -> Result<u8, uicore::Error> {
    u8::try_from(value).map_err(|_| uicore::Error::BadNumber)
}

fn color(cursor: &mut Cursor<'_>) -> Result<[u8; 4], uicore::Error> {
    let red = byte(cursor.u32()?)?;
    cursor.expect(',')?;
    let green = byte(cursor.u32()?)?;
    cursor.expect(',')?;
    let blue = byte(cursor.u32()?)?;
    cursor.expect(',')?;
    let alpha = byte(cursor.u32()?)?;
    Ok([red, green, blue, alpha])
}

fn stop(cursor: &mut Cursor<'_>) -> Result<ColorStop, uicore::Error> {
    let offset = cursor.f32()?;
    if !(0.0..=1.0).contains(&offset) {
        return Err(uicore::Error::BadNumber);
    }
    cursor.expect(',')?;
    Ok(ColorStop {
        offset,
        color: color(cursor)?,
        space: ColorSpace::Srgb,
    })
}

fn stops(cursor: &mut Cursor<'_>, count: u32) -> Result<Vec<ColorStop>, uicore::Error> {
    if count < 2 {
        return Err(uicore::Error::BadNumber);
    }
    let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        cursor.expect(' ')?;
        out.push(stop(cursor)?);
    }
    Ok(out)
}

fn gradient(cursor: &mut Cursor<'_>) -> Result<Gradient, uicore::Error> {
    let kind = cursor.u32()?;
    cursor.expect(' ')?;
    let gradient = match kind {
        1 => {
            let x0 = cursor.f32()?;
            cursor.expect(',')?;
            let y0 = cursor.f32()?;
            cursor.expect(',')?;
            let x1 = cursor.f32()?;
            cursor.expect(',')?;
            let y1 = cursor.f32()?;
            cursor.expect(' ')?;
            let count = cursor.u32()?;
            Gradient::Linear {
                start: [x0, y0],
                end: [x1, y1],
                stops: stops(cursor, count)?,
            }
        }
        2 => {
            let x = cursor.f32()?;
            cursor.expect(',')?;
            let y = cursor.f32()?;
            cursor.expect(',')?;
            let radius = cursor.f32()?;
            cursor.expect(' ')?;
            let count = cursor.u32()?;
            Gradient::Radial {
                center: [x, y],
                radius,
                stops: stops(cursor, count)?,
            }
        }
        3 => {
            let x = cursor.f32()?;
            cursor.expect(',')?;
            let y = cursor.f32()?;
            cursor.expect(',')?;
            let start = cursor.f32()?;
            cursor.expect(',')?;
            let end = cursor.f32()?;
            cursor.expect(' ')?;
            let count = cursor.u32()?;
            Gradient::Sweep {
                center: [x, y],
                start_angle_rad: start,
                end_angle_rad: end,
                stops: stops(cursor, count)?,
            }
        }
        _ => return Err(uicore::Error::BadNumber),
    };
    Ok(gradient)
}

fn run(cursor: &mut Cursor<'_>) -> Result<TextSpan, uicore::Error> {
    let utf8 = cursor.byte_field()?.to_owned();
    let weight = weight(cursor.u32()?)?;
    cursor.expect(' ')?;
    let flags = cursor.u32()?;
    if flags > 3 {
        return Err(uicore::Error::BadNumber);
    }
    cursor.expect(' ')?;
    let family = cursor.byte_field()?;
    let color = color(cursor)?;
    cursor.expect(' ')?;
    let tag = cursor.u32()?;
    let gradient = match tag {
        0 => None,
        1 => {
            cursor.expect(' ')?;
            Some(gradient(cursor)?)
        }
        _ => return Err(uicore::Error::BadNumber),
    };
    cursor.expect('\n')?;
    Ok(TextSpan {
        utf8,
        color,
        gradient,
        weight,
        underline: flags & 1 != 0,
        line_through: flags & 2 != 0,
        family: (!family.is_empty()).then(|| family.to_owned()),
    })
}

/// Parses a `setTextRuns` payload. An empty count clears spans.
pub(crate) fn parse(text: &str) -> Result<Vec<TextSpan>, uicore::Error> {
    let mut cursor = Cursor::new(text);
    let count = cursor.u32()?;
    if count == 0 {
        if cursor.rest() == "\n" || cursor.done() {
            return Ok(Vec::new());
        }
        return Err(uicore::Error::BadNumber);
    }
    cursor.expect('\n')?;
    let mut spans = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        spans.push(run(&mut cursor)?);
    }
    if cursor.done() {
        Ok(spans)
    } else {
        Err(uicore::Error::BadNumber)
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use protocol::{ColorStop, Gradient};

    fn solid(text: &str, color: [u8; 4]) -> String {
        format!(
            "1\n{} {text}400 0 0 {},{},{},{} 0\n",
            text.len(),
            color[0],
            color[1],
            color[2],
            color[3],
        )
    }

    #[test]
    fn a_space_inside_the_run_round_trips() {
        let parsed = parse(&solid("a b", [9, 8, 7, 255])).expect("run");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].utf8, "a b");
        assert_eq!(parsed[0].color, [9, 8, 7, 255]);
        assert!(parsed[0].gradient.is_none());
    }

    #[test]
    fn a_newline_inside_the_run_is_payload() {
        let text = "a\nb";
        let encoded = format!(
            "1\n{} {text}700 3 4 Sans{},{},{},{} 0\n",
            text.len(),
            1,
            2,
            3,
            4
        );
        let parsed = parse(&encoded).expect("run");
        assert_eq!(parsed[0].utf8, text);
        assert_eq!(parsed[0].weight, 700);
        assert!(parsed[0].underline);
        assert!(parsed[0].line_through);
        assert_eq!(parsed[0].family.as_deref(), Some("Sans"));
    }

    #[test]
    fn an_empty_count_clears_spans() {
        assert!(parse("0").expect("clear").is_empty());
        assert!(parse("0\n").expect("clear").is_empty());
    }

    #[test]
    fn a_linear_gradient_round_trips() {
        let encoded = "1\n7 Impasto400 0 0 8,8,8,255 1 1 0,0,1,0 2 0,1,2,3,4 1,5,6,7,8\n";
        let parsed = parse(encoded).expect("gradient");
        assert_eq!(
            parsed[0].gradient,
            Some(Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 0.0],
                stops: vec![
                    ColorStop::srgb(0.0, [1, 2, 3, 4]),
                    ColorStop::srgb(1.0, [5, 6, 7, 8]),
                ],
            })
        );
    }

    #[test]
    fn a_short_gradient_is_rejected() {
        assert!(parse("1\n1 a400 0 0 0,0,0,255 1 1 0,0,1,0 1 0,0,0,0,255\n").is_err());
    }

    #[test]
    fn a_bad_weight_is_rejected() {
        assert!(parse("1\n1 a450 0 0 0,0,0,255 0\n").is_err());
    }
}
