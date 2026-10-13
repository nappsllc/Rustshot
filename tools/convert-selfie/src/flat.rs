//! A minimal FlatBuffers reader, bounds-checked everywhere (the file is
//! downloaded): tables through their vtables, scalars with defaults,
//! offsets to tables, vectors and strings.

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub struct BadFile(pub String);

impl fmt::Display for BadFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BadFile {}

pub type R<T> = Result<T, BadFile>;

pub fn bad<T>(s: impl Into<String>) -> R<T> {
    Err(BadFile(s.into()))
}

/// The buffer.
#[derive(Clone, Copy)]
pub struct Buf<'a>(pub &'a [u8]);

impl<'a> Buf<'a> {
    pub fn bytes(&self, at: usize, n: usize) -> R<&'a [u8]> {
        match at.checked_add(n) {
            Some(end) if end <= self.0.len() => Ok(&self.0[at..end]),
            _ => bad(format!("read of {n} bytes at {at} past the end ({})", self.0.len())),
        }
    }
    pub fn u8(&self, at: usize) -> R<u8> {
        Ok(self.bytes(at, 1)?[0])
    }
    pub fn u16(&self, at: usize) -> R<u16> {
        Ok(u16::from_le_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }
    pub fn u32(&self, at: usize) -> R<u32> {
        Ok(u32::from_le_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }
    pub fn i32(&self, at: usize) -> R<i32> {
        Ok(self.u32(at)? as i32)
    }
    pub fn u64(&self, at: usize) -> R<u64> {
        Ok(u64::from_le_bytes(self.bytes(at, 8)?.try_into().unwrap()))
    }

    /// Follow the uoffset stored at `at`.
    pub fn deref(&self, at: usize) -> R<usize> {
        let off = self.u32(at)? as usize;
        match at.checked_add(off) {
            Some(p) if p < self.0.len() => Ok(p),
            _ => bad(format!("offset at {at} points outside the file")),
        }
    }

    /// The root table.
    pub fn root(&self) -> R<Table<'a>> {
        Table::at(*self, self.deref(0)?)
    }
}

/// A table: its position and its vtable's.
#[derive(Clone, Copy)]
pub struct Table<'a> {
    pub buf: Buf<'a>,
    pub pos: usize,
    vt: usize,
    vt_len: usize,
}

impl<'a> Table<'a> {
    pub fn at(buf: Buf<'a>, pos: usize) -> R<Table<'a>> {
        let soff = buf.i32(pos)? as i64;
        let vt = pos as i64 - soff;
        if vt < 0 || vt as usize >= buf.0.len() {
            return bad(format!("table at {pos}: vtable outside the file"));
        }
        let vt = vt as usize;
        let vt_len = buf.u16(vt)? as usize;
        if vt_len < 4 || !vt_len.is_multiple_of(2) {
            return bad(format!("table at {pos}: vtable of {vt_len} bytes"));
        }
        buf.bytes(vt, vt_len)?;
        Ok(Table { buf, pos, vt, vt_len })
    }

    /// Where field `id` is (absent: `None`).
    pub fn field(&self, id: usize) -> R<Option<usize>> {
        let slot = 4 + 2 * id;
        if slot + 2 > self.vt_len {
            return Ok(None);
        }
        let off = self.buf.u16(self.vt + slot)? as usize;
        Ok((off != 0).then_some(self.pos + off))
    }

    pub fn u8_or(&self, id: usize, d: u8) -> R<u8> {
        self.field(id)?.map_or(Ok(d), |p| self.buf.u8(p))
    }
    pub fn i8_or(&self, id: usize, d: i8) -> R<i8> {
        Ok(self.u8_or(id, d as u8)? as i8)
    }
    pub fn bool_or(&self, id: usize, d: bool) -> R<bool> {
        Ok(self.u8_or(id, d as u8)? != 0)
    }
    pub fn i32_or(&self, id: usize, d: i32) -> R<i32> {
        self.field(id)?.map_or(Ok(d), |p| self.buf.i32(p))
    }
    pub fn u32_or(&self, id: usize, d: u32) -> R<u32> {
        self.field(id)?.map_or(Ok(d), |p| self.buf.u32(p))
    }
    pub fn u64_or(&self, id: usize, d: u64) -> R<u64> {
        self.field(id)?.map_or(Ok(d), |p| self.buf.u64(p))
    }

    pub fn table(&self, id: usize) -> R<Option<Table<'a>>> {
        match self.field(id)? {
            Some(p) => Ok(Some(Table::at(self.buf, self.buf.deref(p)?)?)),
            None => Ok(None),
        }
    }

    /// A vector: its element count and where the elements start.
    pub fn vector(&self, id: usize, elem: usize) -> R<Option<(usize, usize)>> {
        let Some(p) = self.field(id)? else { return Ok(None) };
        let v = self.buf.deref(p)?;
        let n = self.buf.u32(v)? as usize;
        let len = n.checked_mul(elem).ok_or_else(|| BadFile("vector size".into()))?;
        self.buf.bytes(v + 4, len)?;
        Ok(Some((n, v + 4)))
    }

    pub fn bytes(&self, id: usize) -> R<&'a [u8]> {
        match self.vector(id, 1)? {
            Some((n, at)) => self.buf.bytes(at, n),
            None => Ok(&[]),
        }
    }

    pub fn i32s(&self, id: usize) -> R<Vec<i32>> {
        match self.vector(id, 4)? {
            Some((n, at)) => (0..n).map(|i| self.buf.i32(at + 4 * i)).collect(),
            None => Ok(Vec::new()),
        }
    }

    pub fn tables(&self, id: usize) -> R<Vec<Table<'a>>> {
        match self.vector(id, 4)? {
            Some((n, at)) => (0..n).map(|i| Table::at(self.buf, self.buf.deref(at + 4 * i)?)).collect(),
            None => Ok(Vec::new()),
        }
    }

    pub fn string(&self, id: usize) -> R<String> {
        Ok(String::from_utf8_lossy(self.bytes(id)?).into_owned())
    }
}
