use anyhow::{Result, anyhow};
use std::io::Cursor;
use byteorder::ReadBytesExt;
use rustzx_core::zx::machine::ZXMachine;

pub fn convert_z80_to_sna(z80_data: &[u8]) -> Result<(Vec<u8>, ZXMachine)> {
    let mut cursor = Cursor::new(z80_data);
    
    // --- Parse Z80 Header ---
    if z80_data.len() < 30 {
        return Err(anyhow!("Z80 file too short"));
    }
    let a = cursor.read_u8()?;
    let f = cursor.read_u8()?;
    let c = cursor.read_u8()?;
    let b = cursor.read_u8()?;
    let l = cursor.read_u8()?;
    let h = cursor.read_u8()?;
    let pc_low = cursor.read_u8()?;
    let pc_high = cursor.read_u8()?;
    let pc = u16::from_le_bytes([pc_low, pc_high]);
    let sp_low = cursor.read_u8()?;
    let sp_high = cursor.read_u8()?;
    let sp = u16::from_le_bytes([sp_low, sp_high]);
    let i = cursor.read_u8()?;
    let r = cursor.read_u8()?;
    let byte12 = cursor.read_u8()?;
    let byte12 = if byte12 == 255 { 1 } else { byte12 };
    let border = (byte12 >> 1) & 0x07;
    // R register handling (bit 7 of R is in byte12)
    let r_full = (r & 0x7F) | ((byte12 & 0x01) << 7);
    
    let e = cursor.read_u8()?;
    let d = cursor.read_u8()?;
    let c_alt = cursor.read_u8()?;
    let b_alt = cursor.read_u8()?;
    let e_alt = cursor.read_u8()?;
    let d_alt = cursor.read_u8()?;
    let l_alt = cursor.read_u8()?;
    let h_alt = cursor.read_u8()?;
    let a_alt = cursor.read_u8()?;
    let f_alt = cursor.read_u8()?;
    let iy_low = cursor.read_u8()?;
    let iy_high = cursor.read_u8()?;
    let _iy = u16::from_le_bytes([iy_low, iy_high]);
    let ix_low = cursor.read_u8()?;
    let ix_high = cursor.read_u8()?;
    let _ix = u16::from_le_bytes([ix_low, ix_high]);
    let _iff1 = cursor.read_u8()?;
    let iff2 = cursor.read_u8()?;
    let byte29 = cursor.read_u8()?;
    let im = byte29 & 0x03;

    let mut version = 1;
    let mut pc_real = pc;
    let mut port_7ffd = 0;
    let mut machine = ZXMachine::Sinclair48K;
    
    // Check for v2/v3
    if pc == 0 {
        // Extended header
        let len_low = cursor.read_u8()?;
        let len_high = cursor.read_u8()?;
        let header_len = u16::from_le_bytes([len_low, len_high]);
        
        let pc_low_ext = cursor.read_u8()?;
        let pc_high_ext = cursor.read_u8()?;
        pc_real = u16::from_le_bytes([pc_low_ext, pc_high_ext]);
        
        version = match header_len {
            23 => 2,
            54 | 55 => 3,
            _ => return Err(anyhow!("Unsupported Z80 header length {header_len}")),
        };
        let mode = cursor.read_u8()?;
        machine = match (version, mode) {
            (_, 0 | 1) | (3, 3) => ZXMachine::Sinclair48K,
            (2, 3 | 4) | (3, 4..=6) | (_, 12) => ZXMachine::Sinclair128K,
            _ => return Err(anyhow!("Unsupported Z80 hardware mode {mode} (version {version})")),
        };
        port_7ffd = cursor.read_u8()?;
        let end = 32 + usize::from(header_len);
        anyhow::ensure!(end <= z80_data.len(), "Truncated Z80 header");
        cursor.set_position(end as u64);
    }

    // Prepare banks (for 128K support)
    let mut banks = vec![vec![0u8; 16384]; 8];

    // --- Decompress/Copy Memory ---
    if version == 1 {
        let pos_start = cursor.position() as usize;
        let data_rem = &z80_data[pos_start..];
        let compressed = (byte12 & 0x20) != 0;
        
        let mut ram_48k = vec![0u8; 49152];
        if compressed {
            decompress_z80_block(data_rem.strip_suffix(&[0, 0xed, 0xed, 0]).unwrap_or(data_rem), &mut ram_48k)?;
        } else {
            anyhow::ensure!(data_rem.len() == 49152, "Expected 48K of snapshot RAM");
            ram_48k.copy_from_slice(data_rem);
        }
        // Map 48k to banks 5, 2, 0
        banks[5].copy_from_slice(&ram_48k[0..16384]);
        banks[2].copy_from_slice(&ram_48k[16384..32768]);
        banks[0].copy_from_slice(&ram_48k[32768..49152]);
    } else {
        let mut loaded = [false; 8];
        while cursor.position() < z80_data.len() as u64 {
             let len_low = cursor.read_u8()?;
             let len_high = cursor.read_u8()?;
             let block_len = u16::from_le_bytes([len_low, len_high]);
             let page = cursor.read_u8()?;
             
             let data_start = cursor.position() as usize;
             let data_end = if block_len == 0xFFFF { data_start + 16384 } else { data_start + block_len as usize };
             anyhow::ensure!(data_end <= z80_data.len(), "Truncated Z80 RAM block");
             
             let chunk = &z80_data[data_start..data_end];
             cursor.set_position(data_end as u64);

             let bank_idx = if machine == ZXMachine::Sinclair48K {
                 match page {
                     8 => 5,
                     4 => 2,
                     5 => 0,
                     _ => continue,
                 }
             } else {
                 match page {
                     3..=10 => (page - 3) as usize,
                     _ => continue,
                 }
             };

             loaded[bank_idx] = true;
             if block_len == 0xFFFF {
                 banks[bank_idx].copy_from_slice(chunk);
             } else {
                 decompress_z80_block(chunk, &mut banks[bank_idx])?;
             }
        }
        let complete = if machine == ZXMachine::Sinclair48K {
            loaded[5] && loaded[2] && loaded[0]
        } else {
            loaded.iter().all(|present| *present)
        };
        anyhow::ensure!(complete, "Missing Z80 RAM pages");
    }

    // --- Build SNA ---
    let mut sna = Vec::with_capacity(131103);
    sna.push(i);
    sna.push(l_alt); sna.push(h_alt);
    sna.push(e_alt); sna.push(d_alt);
    sna.push(c_alt); sna.push(b_alt);
    sna.push(f_alt); sna.push(a_alt);
    sna.push(l);     sna.push(h);
    sna.push(e);     sna.push(d);
    sna.push(c);     sna.push(b);
    sna.push(iy_low); sna.push(iy_high);
    sna.push(ix_low); sna.push(ix_high);
    sna.push(if iff2 != 0 { 4 } else { 0 });
    sna.push(r_full);
    sna.push(f); sna.push(a);
    sna.push(sp_low); sna.push(sp_high);
    sna.push(im);
    sna.push(border);

    if machine == ZXMachine::Sinclair48K {
        sna.extend_from_slice(&banks[5]);
        sna.extend_from_slice(&banks[2]);
        sna.extend_from_slice(&banks[0]);
        
        let new_sp = sp.wrapping_sub(2);
        sna[23] = (new_sp & 0xFF) as u8;
        sna[24] = (new_sp >> 8) as u8;
        anyhow::ensure!((0x4000..=0xfffe).contains(&new_sp), "Snapshot stack cannot hold the 48K return address");
        let offset = 27 + usize::from(new_sp - 0x4000);
        sna[offset..offset + 2].copy_from_slice(&pc_real.to_le_bytes());
    } else {
        // 128K SNA
        let current_bank = (port_7ffd & 0x07) as usize;
        sna.extend_from_slice(&banks[5]);
        sna.extend_from_slice(&banks[2]);
        sna.extend_from_slice(&banks[current_bank]);
        
        // Extension Header
        sna.push((pc_real & 0xFF) as u8);
        sna.push((pc_real >> 8) as u8);
        sna.push(port_7ffd);
        sna.push(0); // TR-DOS
        
        // Remaining 5 banks
        for (i, bank) in banks.iter().enumerate().take(8) {
            if i != 5 && i != 2 && i != current_bank {
                sna.extend_from_slice(bank);
            }
        }
    }

    Ok((sna, machine))
}

fn decompress_z80_block(input: &[u8], output: &mut [u8]) -> Result<()> {
    let mut i = 0;
    let mut j = 0;
    
    while i < input.len() {
        if input[i..].starts_with(&[0xed, 0xed]) {
            anyhow::ensure!(input.len() - i >= 4, "Incomplete Z80 run");
            let count = usize::from(input[i + 2]);
            anyhow::ensure!(count != 0 && count <= output.len() - j, "Invalid Z80 run length");
            output[j..j + count].fill(input[i + 3]);
            i += 4;
            j += count;
        } else {
            anyhow::ensure!(j < output.len(), "Excess Z80 RAM data");
            output[j] = input[i];
            j += 1;
            i += 1;
        }
    }
    anyhow::ensure!(j == output.len(), "Incomplete Z80 RAM data");
    Ok(())
}
