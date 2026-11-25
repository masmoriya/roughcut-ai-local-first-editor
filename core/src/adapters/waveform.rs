/// Max |sample| per bucket, scaled to u8 — RAW levels, no normalization
/// (analysis wants real energy, not display scaling). Pure.
pub fn raw_peaks(pcm_s16le: &[u8], sample_rate: u32, peaks_per_second: u32) -> Vec<u8> {
    let bucket = (sample_rate / peaks_per_second).max(1) as usize;
    let mut peaks = Vec::with_capacity(pcm_s16le.len() / 2 / bucket + 1);
    let mut max: i32 = 0;
    let mut n = 0usize;
    for chunk in pcm_s16le.chunks_exact(2) {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
        max = max.max(s.abs());
        n += 1;
        if n == bucket {
            peaks.push(((max * 255) / 32768).min(255) as u8);
            max = 0;
            n = 0;
        }
    }
    if n > 0 {
        peaks.push(((max * 255) / 32768).min(255) as u8);
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::raw_peaks;

    #[test]
    fn whole_buckets_can_be_processed_in_chunks() {
        const RATE: u32 = 8000;
        const PEAKS: u32 = 50;
        const CHUNK: usize = (RATE / PEAKS) as usize * 2 * 100;
        let pcm: Vec<u8> = (0..(CHUNK + 324) / 2)
            .flat_map(|index| (index as i16).wrapping_mul(53).to_le_bytes())
            .collect();
        let chunked: Vec<u8> = pcm
            .chunks(CHUNK)
            .flat_map(|chunk| raw_peaks(chunk, RATE, PEAKS))
            .collect();
        assert_eq!(chunked, raw_peaks(&pcm, RATE, PEAKS));
    }
}
