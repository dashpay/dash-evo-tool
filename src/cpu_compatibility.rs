#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use native_dialog::DialogBuilder;

pub fn check_cpu_compatibility() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        use raw_cpuid::CpuId;

        let cpuid = CpuId::new();

        if let Some(feature_info) = cpuid.get_feature_info() {
            if !feature_info.has_avx() {
                DialogBuilder::message()
                    .set_level(native_dialog::MessageLevel::Error)
                    .set_title("Compatibility Error")
                    .set_text(
                        "Your CPU does not support AVX instructions. Please use a compatible CPU.",
                    )
                    .alert()
                    .show()
                    .unwrap();

                std::process::exit(1);
            }
            let avx2_supported = cpuid
                .get_extended_feature_info()
                .is_some_and(|ext_info| ext_info.has_avx2());
            if !avx2_supported {
                DialogBuilder::message()
                    .set_level(native_dialog::MessageLevel::Error)
                    .set_title("Compatibility Error")
                    .set_text(
                        "Your CPU does not support AVX2 instructions. Please use a compatible CPU.",
                    )
                    .alert()
                    .show()
                    .unwrap();
                std::process::exit(1);
            }
        } else {
            DialogBuilder::message()
                .set_level(native_dialog::MessageLevel::Error)
                .set_title("Compatibility Error")
                .set_text("Unable to determine CPU features. Please ensure your CPU is compatible.")
                .alert()
                .show()
                .unwrap();
            std::process::exit(1);
        }
    }
}
