use anyhow::{Result, anyhow};
use std::io::{Cursor, Read};
use byteorder::{ReadBytesExt, LE};
use flate2::read::ZlibDecoder;
use rustzx_core::zx::machine::ZXMachine;

pub fn convert_szx_to_sna(szx_data: &[u8]) -> Result<(Vec<u8>, ZXMachine)> {
    let mut cursor = Cursor::new(szx_data);
    
    // Header (8 bytes)
    let mut signature = [0u8; 4];
    cursor.read_exact(&mut signature)?;
    if &signature != b"ZXST" {
        return Err(anyhow!("Invalid SZX signature"));
    }
    
    let _major = cursor.read_u8()?;
    let _minor = cursor.read_u8()?;
    let machine_id = cursor.read_u8()?;
    let _flags = cursor.read_u8()?;

    let (machine, is_128k) = match machine_id {
        1 | 2 => (ZXMachine::Sinclair48K, false),
        3..=6 => (ZXMachine::Sinclair128K, true),
        _ => return Err(anyhow!("Unsupported machine ID in SZX: {}", machine_id)),
    };
    
    let mut regs = [0u8; 0x25]; // Z80R data size is usually 0x23 or 0x25
    let mut ram_pages: std::collections::HashMap<u8, Vec<u8>> = std::collections::HashMap::new();
    let mut border: u8 = 0;
    let mut port_7ffd: u8 = 0;

    let total_len = szx_data.len() as u64;
    while cursor.position() < total_len {
        let mut tag = [0u8; 4];
        if cursor.read_exact(&mut tag).is_err() { break; }
        let size = cursor.read_u32::<LE>()?;
        let pos_before = cursor.position();
        
        match &tag {
            b"Z80R" => {
                let read_size = std::cmp::min(size as usize, regs.len());
                cursor.read_exact(&mut regs[..read_size])?;
            }
            b"RAMP" => {
                let flags = cursor.read_u16::<LE>()?;
                let page_no = cursor.read_u8()?;
                let data_size = size - 3;
                let mut compressed_data = vec![0u8; data_size as usize];
                cursor.read_exact(&mut compressed_data)?;
                
                let page_data = if flags & 0x01 != 0 {
                    let mut decoder = ZlibDecoder::new(&compressed_data[..]);
                    let mut decompressed = Vec::new();
                    if decoder.read_to_end(&mut decompressed).is_ok() {
                        decompressed
                    } else {
                        return Err(anyhow!("Failed to decompress RAM page {}", page_no));
                    }
                } else {
                    compressed_data
                };
                
                if page_data.len() == 16384 {
                    ram_pages.insert(page_no, page_data);
                }
            }
            b"SPCR" => {
                border = cursor.read_u8()?;
                port_7ffd = cursor.read_u8()?;
            }
            _ => {
                // Skip unknown block
            }
        }
        
        cursor.set_position(pos_before + size as u64);
    }

    if !is_128k {
        // SNA Format (48K)
        let mut sna = Vec::with_capacity(27 + 49152);
        
        let pc_low = regs[0x16];
        let pc_high = regs[0x17];
        let sp_low = regs[0x14];
        let sp_high = regs[0x15];
        let sp_val = u16::from_le_bytes([sp_low, sp_high]);

        sna.push(regs[0x18]); // I
        sna.push(regs[0x0E]); sna.push(regs[0x0F]); // HL'
        sna.push(regs[0x0C]); sna.push(regs[0x0D]); // DE'
        sna.push(regs[0x0A]); sna.push(regs[0x0B]); // BC'
        sna.push(regs[0x08]); sna.push(regs[0x09]); // AF'
        sna.push(regs[0x06]); sna.push(regs[0x07]); // HL
        sna.push(regs[0x04]); sna.push(regs[0x05]); // DE
        sna.push(regs[0x02]); sna.push(regs[0x03]); // BC
        sna.push(regs[0x12]); sna.push(regs[0x13]); // IY
        sna.push(regs[0x10]); sna.push(regs[0x11]); // IX
        
        let iff2 = regs[0x1B];
        sna.push(if iff2 != 0 { 0x04 } else { 0x00 } | 0x02); // IFF2 bit 2, bit 1 set
        
        sna.push(regs[0x19]); // R
        sna.push(regs[0x00]); sna.push(regs[0x01]); // AF

        // We will update SP later
        sna.push(sp_low); sna.push(sp_high); 
        
        sna.push(regs[0x1C]); // IM
        sna.push(border & 0x07); // Border

        // RAM Pages
        let p5 = ram_pages.get(&5).ok_or_else(|| anyhow!("Missing RAM page 5"))?;
        let p2 = ram_pages.get(&2).ok_or_else(|| anyhow!("Missing RAM page 2"))?;
        let p0 = ram_pages.get(&0).ok_or_else(|| anyhow!("Missing RAM page 0"))?;
        
        let mut ram = Vec::with_capacity(49152);
        ram.extend_from_slice(p5);
        ram.extend_from_slice(p2);
        ram.extend_from_slice(p0);

        // Push PC to stack
        let new_sp = sp_val.wrapping_sub(2);
        sna[23] = (new_sp & 0xFF) as u8;
        sna[24] = (new_sp >> 8) as u8;

        if new_sp >= 16384 {
            let ram_offset = (new_sp - 16384) as usize;
            if ram_offset + 1 < ram.len() {
                ram[ram_offset] = pc_low;
                ram[ram_offset + 1] = pc_high;
            }
        }

        sna.extend_from_slice(&ram);

        Ok((sna, machine))
    } else {
        // SNA Format (128K)
        let mut sna = Vec::with_capacity(131103);
        
        // Header (27 bytes)
        sna.push(regs[0x18]); // I
        sna.push(regs[0x0E]); sna.push(regs[0x0F]); // HL'
        sna.push(regs[0x0C]); sna.push(regs[0x0D]); // DE'
        sna.push(regs[0x0A]); sna.push(regs[0x0B]); // BC'
        sna.push(regs[0x08]); sna.push(regs[0x09]); // AF'
        sna.push(regs[0x06]); sna.push(regs[0x07]); // HL
        sna.push(regs[0x04]); sna.push(regs[0x05]); // DE
        sna.push(regs[0x02]); sna.push(regs[0x03]); // BC
        sna.push(regs[0x12]); sna.push(regs[0x13]); // IY
        sna.push(regs[0x10]); sna.push(regs[0x11]); // IX
        
        let iff2 = regs[0x1B];
        sna.push(if iff2 != 0 { 0x04 } else { 0x00 } | 0x02); // IFF2 bit 2
        
        sna.push(regs[0x19]); // R
        sna.push(regs[0x00]); sna.push(regs[0x01]); // AF
        sna.push(regs[0x14]); sna.push(regs[0x15]); // SP
        sna.push(regs[0x1C]); // IM
        sna.push(border & 0x07); // Border

        // RAM Page 5
        let p5 = ram_pages.get(&5).ok_or_else(|| anyhow!("Missing RAM page 5"))?;
        sna.extend_from_slice(p5);
        
        // RAM Page 2
        let p2 = ram_pages.get(&2).ok_or_else(|| anyhow!("Missing RAM page 2"))?;
        sna.extend_from_slice(p2);
        
        // RAM Page n (the one at 0xC000)
        let page_n = port_7ffd & 0x7;
        let p_curr = ram_pages.get(&page_n).ok_or_else(|| anyhow!("Missing current RAM page {}", page_n))?;
        sna.extend_from_slice(p_curr);
        
        // Extension Header
        sna.push(regs[0x16]); // PC Low
        sna.push(regs[0x17]); // PC High
        sna.push(port_7ffd);
        sna.push(0); // TR-DOS
        
        // Remaining 5 pages in numerical order
        for i in 0..8 {
            if i == 5 || i == 2 || i == page_n {
                continue;
            }
            let p = ram_pages.get(&i).ok_or_else(|| anyhow!("Missing RAM page {}", i))?;
            sna.extend_from_slice(p);
        }
        
        Ok((sna, machine))
    }
}
