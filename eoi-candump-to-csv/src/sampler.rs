use std::collections::HashMap;
use std::io::Write;

use anyhow::Context;
use csv::Writer;

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    PerFrame,
    Bucketed { interval_secs: f64 },
}

pub struct Sampler<W: Write> {
    mode: Mode,
    schema: Vec<String>,
    index_by_name: HashMap<String, usize>,
    writer: Writer<W>,
    anchor: Option<f64>,
    current_bucket: Option<u64>,
    current_ts: f64,
    row: Vec<String>,
}

impl<W: Write> Sampler<W> {
    pub fn new(mode: Mode, schema: Vec<String>, mut writer: Writer<W>) -> anyhow::Result<Self> {
        let mut header = Vec::with_capacity(schema.len() + 1);
        header.push("timestamp".to_string());
        header.extend(schema.iter().cloned());
        writer.write_record(&header).context("write CSV header")?;

        let index_by_name = schema
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();
        let row = vec![String::new(); schema.len()];

        Ok(Self {
            mode,
            schema,
            index_by_name,
            writer,
            anchor: None,
            current_bucket: None,
            current_ts: 0.0,
            row,
        })
    }

    pub fn accept(&mut self, timestamp: f64, fields: &[(String, String)]) -> anyhow::Result<()> {
        match self.mode {
            Mode::PerFrame => {
                for cell in &mut self.row {
                    cell.clear();
                }
                for (k, v) in fields {
                    if let Some(&i) = self.index_by_name.get(k) {
                        self.row[i] = v.clone();
                    }
                }
                self.write_row(timestamp)?;
            }
            Mode::Bucketed { interval_secs } => {
                let anchor = *self.anchor.get_or_insert(timestamp);
                let bucket = (((timestamp - anchor) / interval_secs).floor() as i64).max(0) as u64;

                match self.current_bucket {
                    None => {
                        self.current_bucket = Some(bucket);
                        self.current_ts = anchor + bucket as f64 * interval_secs;
                    }
                    Some(prev) if prev != bucket => {
                        let bucket_start = self.current_ts;
                        self.write_row(bucket_start)?;
                        for cell in &mut self.row {
                            cell.clear();
                        }
                        self.current_bucket = Some(bucket);
                        self.current_ts = anchor + bucket as f64 * interval_secs;
                    }
                    _ => {}
                }

                for (k, v) in fields {
                    if let Some(&i) = self.index_by_name.get(k) {
                        self.row[i] = v.clone();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> anyhow::Result<()> {
        if matches!(self.mode, Mode::Bucketed { .. }) && self.current_bucket.is_some() {
            let ts = self.current_ts;
            self.write_row(ts)?;
        }
        self.writer.flush().context("flush CSV writer")?;
        Ok(())
    }

    fn write_row(&mut self, timestamp: f64) -> anyhow::Result<()> {
        let mut record: Vec<&str> = Vec::with_capacity(self.schema.len() + 1);
        let ts_str = format!("{:.6}", timestamp);
        record.push(&ts_str);
        for cell in &self.row {
            record.push(cell.as_str());
        }
        self.writer.write_record(&record).context("write CSV row")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csv::WriterBuilder;

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    fn collect_csv<W: AsRef<[u8]>>(buf: W) -> Vec<Vec<String>> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(buf.as_ref());
        rdr.records()
            .map(|r| r.unwrap().iter().map(String::from).collect())
            .collect()
    }

    #[test]
    fn bucketed_emits_two_rows_across_boundary() {
        let mut writer_buf = Vec::new();
        {
            let writer = WriterBuilder::new().from_writer(&mut writer_buf);
            let schema = vec!["a".to_string(), "b".to_string()];
            let mut s =
                Sampler::new(Mode::Bucketed { interval_secs: 1.0 }, schema, writer).unwrap();
            s.accept(1000.0, &[pair("a", "1")]).unwrap();
            s.accept(1000.5, &[pair("a", "2"), pair("b", "x")]).unwrap();
            s.accept(1001.2, &[pair("a", "9")]).unwrap();
            s.finish().unwrap();
        }
        let rows = collect_csv(&writer_buf);
        // header + 2 data rows
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["timestamp", "a", "b"]);
        assert_eq!(rows[1][1], "2"); // last value in first bucket
        assert_eq!(rows[1][2], "x");
        assert_eq!(rows[2][1], "9"); // value in second bucket
        assert_eq!(rows[2][2], ""); // not updated in second bucket
    }

    #[test]
    fn per_frame_emits_row_per_call() {
        let mut writer_buf = Vec::new();
        {
            let writer = WriterBuilder::new().from_writer(&mut writer_buf);
            let schema = vec!["a".to_string(), "b".to_string()];
            let mut s = Sampler::new(Mode::PerFrame, schema, writer).unwrap();
            s.accept(1.0, &[pair("a", "1")]).unwrap();
            s.accept(1.001, &[pair("b", "y")]).unwrap();
            s.finish().unwrap();
        }
        let rows = collect_csv(&writer_buf);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "1");
        assert_eq!(rows[1][2], "");
        assert_eq!(rows[2][1], "");
        assert_eq!(rows[2][2], "y");
    }
}
