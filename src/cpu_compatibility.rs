#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use native_dialog::MessageDialog;

pub fn check_cpu_compatibility() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        use raw_cpuid::CpuId;

        let cpuid = CpuId::new();

        if let Some(feature_info) = cpuid.get_feature_info() {
            if !feature_info.has_avx() {
                MessageDialog::new()
                    .set_type(native_dialog::MessageType::Error)
                    .set_title("Compatibility Error")
                    .set_text(
                        "Your CPU does not support AVX instructions. Please use a compatible CPU.",
                    )
                    .show_alert()
                    .unwrap();
                std::process::exit(1);
            }
            let avx2_supported = cpuid
                .get_extended_feature_info()
                .map_or(false, |ext_info| ext_info.has_avx2());
            if !avx2_supported {
                MessageDialog::new()
                    .set_type(native_dialog::MessageType::Error)
                    .set_title("Compatibility Error")
                    .set_text(
                        "Your CPU does not support AVX2 instructions. Please use a compatible CPU.",
                    )
                    .show_alert()
                    .unwrap();
                std::process::exit(1);
            }
        } else {
            MessageDialog::new()
                .set_type(native_dialog::MessageType::Error)
                .set_title("Compatibility Error")
                .set_text("Unable to determine CPU features.")
                .show_alert()
                .unwrap();
            std::process::exit(1);
        }
    }
}
