//! Parameter sets survive media quarantine, never SSRC/session changes. A fresh
//! IDR may reference sets sent separately; do not require the sender to repeat
//! them in the same AU. No picture/reference data is cached here.
use h264_reader::{
    Context,
    annexb::AnnexBReader,
    nal::{Nal, RefNal, UnitType, pps::PicParameterSet, sps::SeqParameterSet},
    push::NalInterest,
    rbsp::BitRead,
};
use std::{collections::BTreeMap, io::Read};

const MAX_PARAMETER_BYTES: usize = 128 * 1024;

#[derive(Default)]
pub(super) struct Parameters {
    context: Context,
    sps: BTreeMap<u8, Vec<u8>>,
    pps: BTreeMap<u8, Vec<u8>>,
}

impl Parameters {
    pub(super) fn observe(&mut self, data: &[u8]) {
        let mut reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
            let Ok(header) = nal.header() else {
                return NalInterest::Ignore;
            };
            let kind = header.nal_unit_type();
            if !matches!(kind, UnitType::SeqParameterSet | UnitType::PicParameterSet) {
                return NalInterest::Ignore;
            }
            if !nal.is_complete() {
                return NalInterest::Buffer;
            }
            let mut bytes = Vec::new();
            if nal
                .reader()
                .take(MAX_PARAMETER_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .is_err()
                || bytes.len() > MAX_PARAMETER_BYTES
            {
                *self = Self::default();
                return NalInterest::Ignore;
            }
            let used: usize = self
                .sps
                .values()
                .chain(self.pps.values())
                .map(Vec::len)
                .sum();
            if kind == UnitType::SeqParameterSet {
                if let Ok(sps) = SeqParameterSet::from_bits(nal.rbsp_bits()) {
                    let id = sps.seq_parameter_set_id.id();
                    let previous = self.sps.get(&id).map_or(0, Vec::len);
                    if used - previous + bytes.len() <= MAX_PARAMETER_BYTES {
                        self.sps.insert(id, bytes);
                        self.context.put_seq_param_set(sps);
                    } else {
                        *self = Self::default();
                    }
                } else {
                    *self = Self::default();
                }
            } else if let Ok(pps) = PicParameterSet::from_bits(&self.context, nal.rbsp_bits()) {
                let id = pps.pic_parameter_set_id.id();
                let previous = self.pps.get(&id).map_or(0, Vec::len);
                if used - previous + bytes.len() <= MAX_PARAMETER_BYTES {
                    self.pps.insert(id, bytes);
                } else {
                    *self = Self::default();
                }
            } else {
                *self = Self::default();
            }
            NalInterest::Ignore
        });
        reader.push(data);
        reader.reset();
    }

    /// Prefix exactly the sets referenced by every IDR slice, or nothing when
    /// the AU already carries them (SPS before PPS) ahead of its first IDR
    /// slice: a complete IDR stays byte-identical. Parsing the slice prefix
    /// proves parameter identity, not full hardware decodability.
    pub(super) fn prefix_for_idr(&self, data: &[u8]) -> Option<Vec<u8>> {
        let mut required = Vec::new();
        let mut invalid = false;
        let mut slice_seen = false;
        let mut carried_sps = Vec::new();
        let mut carried_pps = Vec::new();
        let mut reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
            let Ok(header) = nal.header() else {
                return NalInterest::Ignore;
            };
            match header.nal_unit_type() {
                UnitType::SliceLayerWithoutPartitioningIdr => {
                    slice_seen = true;
                    let mut bits = nal.rbsp_bits();
                    let id = bits
                        .read_ue("first_mb_in_slice")
                        .and_then(|_| bits.read_ue("slice_type"))
                        .and_then(|_| bits.read_ue("pic_parameter_set_id"));
                    match id {
                        Ok(id) if id <= 255 => {
                            if !required.contains(&(id as u8)) {
                                required.push(id as u8);
                            }
                        }
                        _ => invalid = true,
                    }
                }
                kind @ (UnitType::SeqParameterSet | UnitType::PicParameterSet) if !slice_seen => {
                    if !nal.is_complete() {
                        return NalInterest::Buffer;
                    }
                    let mut bits = nal.rbsp_bits();
                    if kind == UnitType::SeqParameterSet {
                        // profile_idc, constraint flags and level_idc precede the id.
                        if let Ok(id) = bits
                            .skip(24, "profile_constraints_level")
                            .and_then(|_| bits.read_ue("seq_parameter_set_id"))
                        {
                            carried_sps.push(id);
                        }
                    } else if let Ok(pps) = bits.read_ue("pic_parameter_set_id")
                        && let Ok(sps) = bits.read_ue("seq_parameter_set_id")
                        && carried_sps.contains(&sps)
                    {
                        carried_pps.push(pps);
                    }
                }
                _ => {}
            }
            NalInterest::Ignore
        });
        reader.push(data);
        reader.reset();
        if invalid || required.is_empty() {
            return None;
        }
        if required
            .iter()
            .all(|id| carried_pps.contains(&u32::from(*id)))
        {
            return Some(Vec::new());
        }
        let mut prefix = Vec::new();
        let mut added_sps = Vec::new();
        for id in required {
            let raw_pps = self.pps.get(&id)?;
            let nal = RefNal::new(raw_pps, &[], true);
            // Re-parse against current SPS: reusing an id must not bless a PPS
            // that was only valid with the previous sequence parameters.
            let pps = PicParameterSet::from_bits(&self.context, nal.rbsp_bits()).ok()?;
            let sps_id = pps.seq_parameter_set_id.id();
            if !added_sps.contains(&sps_id) {
                prefix.extend_from_slice(&[0, 0, 0, 1]);
                prefix.extend_from_slice(self.sps.get(&sps_id)?);
                added_sps.push(sps_id);
            }
            prefix.extend_from_slice(&[0, 0, 0, 1]);
            prefix.extend_from_slice(raw_pps);
        }
        Some(prefix)
    }
}
