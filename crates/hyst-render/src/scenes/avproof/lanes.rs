//! What kind of instrument a lane is, worked out from its notes alone.
//!
//! Every threshold lives here. A lane is **sustained** (drawn as a flowing
//! ribbon) only when all of these hold:
//! - its name names no drum ([`drum_name`]);
//! - its pitch moves: at least [`MIN_PITCHES`] distinct pitches, and the
//!   pitch changes between at least [`MIN_PITCH_MOVES`] of consecutive notes.
//!   A lane of one single pitch is exempt: held, it is a drone, drawn as a
//!   flat melody ribbon;
//! - its notes are long or legato: the time-weighted mean note length is at
//!   least [`LONG_NOTE`] seconds, or the median note is at least
//!   [`LEGATO_MIN_LEN`] seconds and lasts at least [`LEGATO_FILL`] of the gap
//!   to the next note.
//!
//! Everything else, including every lane that is ambiguous between the two,
//! is **percussive**: its hits push the scene around instead.
//!
//! A sustained lane is **chordal** (drawn as a band of rings around the core,
//! not a melody ribbon) when more than [`CHORD_SHARE`] of its onsets carry two
//! or more notes. Notes whose onsets lie within [`ONSET_TOGETHER`] seconds of
//! the first note of a group count as one onset. A lane mixing chords and
//! single notes is therefore chordal only if most of its onsets are chords.
//!
//! A melody ribbon maps its lane's own pitch range onto its band's height.
//! A lane narrower than [`MELODY_MIN_RANGE`] semitones is mapped as if it
//! spanned that many, centred: a 2-semitone lane moves over 2/6 of its band,
//! never less. A lane of a single pitch is a flat line at the band's centre.

/// Distinct pitches a sustained lane needs.
pub const MIN_PITCHES: usize = 3;
/// Share of consecutive note pairs whose pitch differs.
pub const MIN_PITCH_MOVES: f32 = 0.3;
/// Time-weighted mean note length, seconds, above which a lane holds notes.
pub const LONG_NOTE: f32 = 0.5;
/// Median note length, seconds, a legato lane needs.
pub const LEGATO_MIN_LEN: f32 = 0.4;
/// Median share of the gap to the next note a legato note fills.
pub const LEGATO_FILL: f32 = 0.9;
/// Share of onsets that must be chords (2+ notes) for a lane to be chordal.
pub const CHORD_SHARE: f32 = 0.5;
/// Onsets closer than this, seconds, start the same chord.
pub const ONSET_TOGETHER: f32 = 0.03;
/// Narrowest pitch range, semitones, a melody band is scaled to.
pub const MELODY_MIN_RANGE: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub t: f32,
    pub end: f32,
    pub pitch: u8,
    pub vel: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Character {
    /// Held single notes: a melody ribbon.
    Sustained,
    /// Held chords: a band of rings.
    Chordal,
    Percussive,
}

/// How a percussive lane pushes the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drum {
    /// Outward from the core, the whole field breathes.
    Kick,
    /// Snare, clap, rim: a twist around the core.
    Snare,
    /// Hats, cymbals, hiss: a fine shiver.
    Hat,
    /// Toms: a shove from one side.
    Tom,
    /// Anything else: a push out from the spectrum ring at its pitch.
    Pitched,
}

/// The drum a lane's name names, if any. Names are split into words; short
/// abbreviations must match a whole word, longer ones only its start.
pub fn drum_name(name: &str) -> Option<Drum> {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let any = |whole: &[&str], prefix: &[&str]| {
        words
            .iter()
            .any(|w| whole.contains(w) || prefix.iter().any(|p| w.starts_with(p)))
    };
    if any(&["bd"], &["kick", "generator"]) {
        Some(Drum::Kick)
    } else if any(&["sn", "sd", "rim"], &["snare", "clap"]) {
        Some(Drum::Snare)
    } else if any(
        &["oh", "ch", "hh"],
        &["hat", "hiss", "cymbal", "crash", "ride", "shaker"],
    ) {
        Some(Drum::Hat)
    } else if any(&[], &["tom"]) {
        Some(Drum::Tom)
    } else if any(&[], &["perc", "drum"]) {
        Some(Drum::Pitched)
    } else {
        None
    }
}

/// The character of a lane from its name and notes (sorted by onset).
pub fn classify(name: &str, notes: &[Note]) -> Character {
    if drum_name(name).is_some() || notes.is_empty() {
        return Character::Percussive;
    }
    let mut pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    pitches.sort_unstable();
    pitches.dedup();
    if pitches.len() > 1 {
        // Two pitches mean at least two notes.
        let moves = notes
            .windows(2)
            .filter(|w| w[0].pitch != w[1].pitch)
            .count() as f32
            / (notes.len() - 1) as f32;
        if pitches.len() < MIN_PITCHES || moves < MIN_PITCH_MOVES {
            return Character::Percussive;
        }
    }
    let lens: Vec<f32> = notes.iter().map(|n| (n.end - n.t).max(0.0)).collect();
    let total: f32 = lens.iter().sum();
    let weighted = if total > 0.0 {
        lens.iter().map(|l| l * l).sum::<f32>() / total
    } else {
        0.0
    };
    let mut fills: Vec<f32> = notes
        .windows(2)
        .filter(|w| w[1].t - w[0].t > 1e-3)
        .map(|w| (w[0].end - w[0].t) / (w[1].t - w[0].t))
        .collect();
    fills.sort_by(f32::total_cmp);
    let mut sorted = lens.clone();
    sorted.sort_by(f32::total_cmp);
    let median_len = sorted[sorted.len() / 2];
    let median_fill = fills.get(fills.len() / 2).copied().unwrap_or(0.0);
    if weighted < LONG_NOTE && (median_len < LEGATO_MIN_LEN || median_fill < LEGATO_FILL) {
        return Character::Percussive;
    }
    let (mut onsets, mut chords) = (0usize, 0usize);
    let mut i = 0;
    while i < notes.len() {
        let j = i + notes[i..]
            .iter()
            .take_while(|n| n.t - notes[i].t <= ONSET_TOGETHER)
            .count();
        onsets += 1;
        chords += usize::from(j - i >= 2);
        i = j;
    }
    if chords as f32 > CHORD_SHARE * onsets as f32 {
        Character::Chordal
    } else {
        Character::Sustained
    }
}

/// The drum a percussive lane plays: its name, else a pitched push.
pub fn drum(name: &str) -> Drum {
    drum_name(name).unwrap_or(Drum::Pitched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(spec: &[(f32, f32, u8)]) -> Vec<Note> {
        spec.iter()
            .map(|&(t, len, pitch)| Note {
                t,
                end: t + len,
                pitch,
                vel: 0.8,
            })
            .collect()
    }

    #[test]
    fn long_legato_moving_pitch_is_sustained() {
        // A flute line: two-second notes, each running into the next.
        let flute: Vec<_> = (0..12)
            .map(|i| (i as f32 * 2.0, 2.0, [60, 63, 67, 65, 62][i % 5]))
            .collect();
        assert_eq!(classify("MAIN SYNTH", &notes(&flute)), Character::Sustained);
        // Legato quarter-second-plus notes that fill their gaps.
        let legato: Vec<_> = (0..16)
            .map(|i| (i as f32 * 0.45, 0.45, 60 + (i % 4) as u8 * 2))
            .collect();
        assert_eq!(classify("Lead", &notes(&legato)), Character::Sustained);
    }

    #[test]
    fn short_hits_and_drum_names_are_percussive() {
        // Short repeated hits on one pitch.
        let hits: Vec<_> = (0..32).map(|i| (i as f32 * 0.25, 0.06, 36)).collect();
        assert_eq!(classify("Thing", &notes(&hits)), Character::Percussive);
        // Short notes with a moving pitch: a sequencer, not a held line.
        let seq: Vec<_> = (0..32)
            .map(|i| (i as f32 * 0.25, 0.12, 48 + (i % 3) as u8))
            .collect();
        assert_eq!(
            classify("MAIN SYNTH #2", &notes(&seq)),
            Character::Percussive
        );
        // A drum name wins even over long notes with a moving pitch.
        let long: Vec<_> = (0..8)
            .map(|i| (i as f32 * 2.0, 2.0, 40 + i as u8))
            .collect();
        for name in ["Perc 1 · TOM1", "Hats", "Perc 1 · OH", "Generator", "Clap"] {
            assert_eq!(
                classify(name, &notes(&long)),
                Character::Percussive,
                "{name}"
            );
        }
    }

    /// One held pitch is a drone: sustained, so a flat melody ribbon.
    #[test]
    fn a_held_single_pitch_is_sustained() {
        let drone: Vec<_> = (0..6).map(|i| (i as f32 * 2.0, 2.0, 57)).collect();
        assert_eq!(classify("Pad", &notes(&drone)), Character::Sustained);
        assert_eq!(
            classify("Pad", &notes(&[(0.0, 4.0, 57)])),
            Character::Sustained
        );
    }

    /// Chords on most onsets make a lane chordal; a few chords in a line
    /// do not.
    #[test]
    fn chords_on_most_onsets_are_chordal() {
        let chord = |t: f32, root: u8| {
            [
                (t, 2.0, root),
                (t, 2.0, root + 4),
                (t + 0.01, 2.0, root + 7),
            ]
        };
        let pad: Vec<_> = (0..6)
            .flat_map(|i| chord(i as f32 * 2.0, 60 + i as u8))
            .collect();
        assert_eq!(classify("MAIN SYNTH", &notes(&pad)), Character::Chordal);
        // Three chords and four single notes: mostly single, a melody.
        let mut mixed: Vec<_> = (0..3)
            .flat_map(|i| chord(i as f32 * 4.0, 60 + i as u8))
            .collect();
        mixed.extend((0..4).map(|i| (i as f32 * 4.0 + 2.0, 2.0, 70 + i as u8)));
        mixed.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(classify("Keys", &notes(&mixed)), Character::Sustained);
        // Four chords and three single notes: mostly chords.
        let mut mostly: Vec<_> = (0..4)
            .flat_map(|i| chord(i as f32 * 4.0, 60 + i as u8))
            .collect();
        mostly.extend((0..3).map(|i| (i as f32 * 4.0 + 2.0, 2.0, 70 + i as u8)));
        mostly.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(classify("Keys", &notes(&mostly)), Character::Chordal);
    }

    #[test]
    fn drum_names_push_differently() {
        assert_eq!(drum("KICK"), Drum::Kick);
        assert_eq!(drum("Generator (Kick)"), Drum::Kick);
        assert_eq!(drum("Perc 1 · BD"), Drum::Kick);
        assert_eq!(drum("Perc 1 · SN"), Drum::Snare);
        assert_eq!(drum("Perc 1 · RIM"), Drum::Snare);
        assert_eq!(drum("Clap"), Drum::Snare);
        assert_eq!(drum("Hats"), Drum::Hat);
        assert_eq!(drum("Perc 1 · OH"), Drum::Hat);
        assert_eq!(drum("Hiss"), Drum::Hat);
        assert_eq!(drum("Perc 1 · TOM3"), Drum::Tom);
        assert_eq!(drum("SECONDARY SYNTH/CHRONOS"), Drum::Pitched);
        assert_eq!(drum("MAIN SYNTH"), Drum::Pitched);
    }
}
