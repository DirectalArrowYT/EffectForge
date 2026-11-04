use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 4 && args[1] == "dump" {
        let input_file = &args[2];
        let output_dir = &args[3];
        let data = fs::read(input_file).unwrap();
        let namco = effect_library::NamcoEffectFile::load(&data).unwrap();
        effect_library::Dumper::dump_namco(&namco, output_dir).unwrap();
        println!("Dump completed to {}", output_dir);
        return;
    }

    if args.len() == 4 && args[1] == "build" {
        let input_dir = &args[2];
        let output_file = &args[3];
        match effect_library::Creator::create_namco_from_folder(input_dir) {
            Ok(Some(namco)) => {
                fs::write(output_file, namco.save().unwrap()).unwrap();
                println!("Build completed to {}", output_file);
            }
            Ok(None) => {
                println!("Build skipped (header-only effect, no Base.ptcl)");
            }
            Err(err) => {
                eprintln!("Build failed: {}", err);
                std::process::exit(1);
            }
        }
        return;
    }

    eprintln!(
        "Usage:\n  {} dump <input.eff> <output_dir>\n  {} build <decompiled_dir> <output.eff>",
        Path::new(&args[0]).file_name().unwrap().to_string_lossy(),
        Path::new(&args[0]).file_name().unwrap().to_string_lossy()
    );
}

#[cfg(test)]
mod tests {
    use effect_library::{BfresFile, BnshFile};

    #[test]
    fn test_bnsh_export() {
        let mut bnsh_file = BnshFile::new("BNSH", 1);
        bnsh_file.add_variation("Shader_0", vec![0x42, 0x42, 0x42, 0x42]);
        let serialized_data = bnsh_file.serialize();

        assert!(!serialized_data.is_empty());
    }

    #[test]
    fn test_bfres_export() {
        let mut bfres_file = BfresFile::new("BFRES", 1);
        bfres_file.add_model("Model_0", vec![0x42, 0x42, 0x42, 0x42]);
        let serialized_data = bfres_file.serialize();

        assert!(!serialized_data.is_empty());
    }
}
