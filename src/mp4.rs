//! Index « mfra » synthétisé pour les MP4 fragmentés de YouTube (DASH).
//! YouTube place un « sidx » (taille et durée de chaque fragment) en tête de fichier, mais pas de
//! « mfra » en fin. Le lecteur MP4 de Windows cherche ce dernier ; faute de le trouver, il parcourt
//! tous les fragments avant de lire (démarrage et sauts interminables). On le reconstruit.

fn be32(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
fn be64(d: &[u8], o: usize) -> Option<u64> {
    d.get(o..o + 8).map(|b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// track_ID du premier « tkhd » trouvé dans `moov`.
fn track_id(moov: &[u8]) -> Option<u32> {
    let i = moov.windows(4).position(|w| w == b"tkhd")?;
    let v = *moov.get(i + 4)?;
    be32(moov, i + 8 + if v == 1 { 16 } else { 8 })
}

pub fn mfra_from_head(d: &[u8]) -> Option<Vec<u8>> {
    let mut off = 0usize;
    let mut track = 1u32;
    while off.saturating_add(8) <= d.len() {
        let mut size = be32(d, off)? as u64;
        let typ = &d[off + 4..off + 8];
        let mut hdr = 8usize;
        if size == 1 {
            size = be64(d, off + 8)?;
            hdr = 16;
        }
        if size < 8 {
            return None;
        }
        let Some(end) = usize::try_from(size).ok().and_then(|s| off.checked_add(s)) else { return None };
        match typ {
            b"moov" => track = track_id(d.get(off..end.min(d.len()))?).unwrap_or(1),
            b"sidx" => {
                let s = d.get(off..end)?; // le sidx doit tenir entièrement dans l'en-tête lu
                let p = hdr;
                let v = *s.get(p)?;
                let timescale = be32(s, p + 8)?;
                let (ept, first, mut q) = if v == 0 { (be32(s, p + 12)? as u64, be32(s, p + 16)? as u64, p + 20) } else { (be64(s, p + 12)?, be64(s, p + 20)?, p + 28) };
                q += 2; // réservé
                let n = u16::from_be_bytes([*s.get(q)?, *s.get(q + 1)?]) as usize;
                q += 2;
                let _ = timescale;
                let mut moof = (end as u64).checked_add(first)?;
                let mut t = ept;
                let mut entries = Vec::with_capacity(n);
                for i in 0..n {
                    let r = be32(s, q + i * 12)?;
                    let dur = be32(s, q + i * 12 + 4)? as u64;
                    if r & 0x8000_0000 != 0 {
                        return None; // référence vers un autre sidx : non géré
                    }
                    entries.push((t, moof));
                    moof = moof.checked_add((r & 0x7FFF_FFFF) as u64)?;
                    t = t.checked_add(dur)?;
                }
                return Some(build(track, &entries));
            }
            _ => {}
        }
        off = end;
    }
    None
}

fn build(track: u32, entries: &[(u64, u64)]) -> Vec<u8> {
    let tfra_size = 12 + 4 + 4 + 4 + entries.len() * 19;
    let mfra_size = 8 + tfra_size + 16;
    let mut o = Vec::with_capacity(mfra_size);
    o.extend((mfra_size as u32).to_be_bytes());
    o.extend(b"mfra");
    o.extend((tfra_size as u32).to_be_bytes());
    o.extend(b"tfra");
    o.extend(0x0100_0000u32.to_be_bytes()); // version 1, flags 0
    o.extend(track.to_be_bytes());
    o.extend(0u32.to_be_bytes()); // traf/trun/sample number sur 1 octet
    o.extend((entries.len() as u32).to_be_bytes());
    for (t, m) in entries {
        o.extend(t.to_be_bytes());
        o.extend(m.to_be_bytes());
        o.extend([1u8, 1, 1]);
    }
    o.extend(16u32.to_be_bytes());
    o.extend(b"mfro");
    o.extend(0u32.to_be_bytes());
    o.extend((mfra_size as u32).to_be_bytes());
    o
}

#[cfg(test)]
mod tests {
    use super::mfra_from_head;

    fn sidx(n_bytes_missing: usize) -> Vec<u8> {
        let mut d = vec![0, 0, 0, 16];
        d.extend(b"ftyp");
        d.extend([0u8; 8]);
        let mut s = Vec::new();
        s.extend([0u8; 4]); // version 0, flags
        s.extend(1u32.to_be_bytes()); // reference_ID
        s.extend(90_000u32.to_be_bytes()); // timescale
        s.extend(0u32.to_be_bytes()); // earliest_presentation_time
        s.extend(0u32.to_be_bytes()); // first_offset
        s.extend([0u8; 2]); // réservé
        s.extend(1u16.to_be_bytes()); // 1 référence
        s.extend(1000u32.to_be_bytes()); // taille du fragment
        s.extend(90_000u32.to_be_bytes()); // durée
        s.extend([0u8; 4]); // SAP
        d.extend(((s.len() + 8) as u32).to_be_bytes());
        d.extend(b"sidx");
        d.extend(s);
        d.truncate(d.len() - n_bytes_missing);
        d
    }

    #[test]
    fn sidx_complet() {
        assert!(mfra_from_head(&sidx(0)).is_some());
    }

    #[test]
    fn entete_tronque_ou_absurde() {
        assert!(mfra_from_head(&[]).is_none());
        for k in [1, 5, 12, 20] {
            assert!(mfra_from_head(&sidx(k)).is_none(), "tronqué de {k} octets");
        }
        // taille 64 bits énorme : ni panique ni dépassement
        let mut e = vec![0, 0, 0, 1];
        e.extend(b"sidx");
        e.extend(u64::MAX.to_be_bytes());
        assert!(mfra_from_head(&e).is_none());
    }
}
