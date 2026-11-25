pub(crate) fn parse_timecode(value: &str, nominal: i64, fractional: bool) -> Option<(i64, bool)> {
    if !(1..=i64::from(u32::MAX)).contains(&nominal) {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 11
        || !bytes.iter().enumerate().all(|(index, byte)| match index {
            2 | 5 => *byte == b':',
            8 => matches!(*byte, b':' | b';'),
            _ => byte.is_ascii_digit(),
        })
    {
        return None;
    }
    let component =
        |start: usize| i64::from(bytes[start] - b'0') * 10 + i64::from(bytes[start + 1] - b'0');
    let (hours, minutes, seconds, frames) =
        (component(0), component(3), component(6), component(9));
    if hours >= 24 || minutes >= 60 || seconds >= 60 || frames >= nominal {
        return None;
    }
    let drop_frame = bytes[8] == b';';
    let mut frame = ((hours * 60 + minutes) * 60 + seconds) * nominal + frames;
    if drop_frame {
        if !fractional || !matches!(nominal, 30 | 60) {
            return None;
        }
        let drop_count = nominal / 15;
        if minutes % 10 != 0 && seconds == 0 && frames < drop_count {
            return None;
        }
        let total_minutes = hours * 60 + minutes;
        frame -= drop_count * (total_minutes - total_minutes / 10);
    }
    Some((frame, drop_frame))
}

#[cfg(test)]
mod tests {
    use super::parse_timecode;

    #[test]
    fn validates_drop_frame_minute_boundaries() {
        assert_eq!(parse_timecode("00:00:59;29", 30, true), Some((1799, true)));
        assert_eq!(parse_timecode("00:01:00;02", 30, true), Some((1800, true)));
        assert_eq!(parse_timecode("00:01:00;01", 30, true), None);
        assert_eq!(parse_timecode("00:10:00;00", 30, true), Some((17982, true)));
        assert_eq!(parse_timecode("00:01:00;04", 60, true), Some((3600, true)));
        assert_eq!(parse_timecode("00:00:00:00", 30, true), Some((0, false)));
        assert_eq!(parse_timecode("23:59:59:00", i64::MAX, false), None);
        assert_eq!(parse_timecode("00:00:00:00", 0, false), None);
    }
}
