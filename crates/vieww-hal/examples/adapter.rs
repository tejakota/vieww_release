//! Which adapter `VulkanDevice::new` actually picked — and which ones it had
//! to choose from.
//!
//! `cargo run --release -p vieww-hal --example adapter --features vulkan`
//!
//! # Why this exists
//!
//! Two reasons, and the second one turned out to matter more than the first.
//!
//! **A skipped test is not a passing test.** Every Vulkan suite in this crate
//! is `#[ignore]`d and *skips* when no device is available: `try_device`
//! returns `None` and the test returns early, green. So a `--ignored` run on a
//! machine with no working ICD reports success and proves nothing. This exits
//! non-zero instead.
//!
//! **A machine can have more than one GPU.** `VulkanDevice::new` takes the
//! first physical device with a graphics queue family, in whatever order the
//! loader enumerated them — no scoring, no discrete-GPU preference, no
//! override. On a laptop with switchable graphics that silently means the
//! integrated part; on a machine where a software ICD happens to enumerate
//! first it means llvmpipe, at which point every "GPU" number measured is a CPU
//! number wearing a Vulkan interface. Neither announces itself. So this lists
//! every candidate and marks the one the framework will use, which turns a
//! silent policy into a visible one.

use std::ffi::CStr;

use vieww_hal::vulkan::VulkanDevice;
use vieww_hal::Device as _;

fn main() {
    enumerate();

    match VulkanDevice::new() {
        Ok(device) => {
            let info = device.info();
            println!();
            println!("name={}", info.name);
            println!("type={}", info.device_type);
            println!("backend={}", info.backend);
            let lowered = info.name.to_lowercase();
            if lowered.contains("llvmpipe")
                || lowered.contains("lavapipe")
                || lowered.contains("swiftshader")
            {
                println!("verdict=SOFTWARE");
            } else {
                println!("verdict=HARDWARE");
            }
            if info.device_type.contains("INTEGRATED") {
                println!("note=integrated GPU; a discrete one may be present and unused");
            }
        }
        Err(error) => {
            eprintln!("no vulkan device: {error}");
            std::process::exit(1);
        }
    }
}

/// Every physical device the loader offers, in enumeration order — which is
/// also selection order, since `pick_physical_device` takes the first match.
fn enumerate() {
    let entry = match vieww_hal::vulkan::load_entry() {
        Ok(entry) => entry,
        Err(error) => {
            eprintln!("no vulkan loader: {error}");
            return;
        }
    };
    let app = ash::vk::ApplicationInfo::default().api_version(ash::vk::API_VERSION_1_2);
    let create = ash::vk::InstanceCreateInfo::default().application_info(&app);
    let instance = match unsafe { entry.create_instance(&create, None) } {
        Ok(instance) => instance,
        Err(error) => {
            eprintln!("could not create an instance: {error}");
            return;
        }
    };

    println!("candidates, in the order the loader returns them:");
    let devices = unsafe { instance.enumerate_physical_devices() }.unwrap_or_default();
    for (index, pd) in devices.iter().enumerate() {
        let props = unsafe { instance.get_physical_device_properties(*pd) };
        let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy();
        let families = unsafe { instance.get_physical_device_queue_family_properties(*pd) };
        let graphics = families
            .iter()
            .any(|f| f.queue_flags.contains(ash::vk::QueueFlags::GRAPHICS));
        // What an unscored `find_map` over this list would have taken. Marked
        // for contrast, not as a claim: the line below the list is what the
        // framework actually selected, and if the two differ that is the
        // selection fix doing its job.
        let first_match = graphics
            && !devices.iter().take(index).any(|earlier| {
                unsafe { instance.get_physical_device_queue_family_properties(*earlier) }
                    .iter()
                    .any(|f| f.queue_flags.contains(ash::vk::QueueFlags::GRAPHICS))
            });
        println!(
            "  [{index}] {name}  {:?}{}{}",
            props.device_type,
            if graphics {
                ""
            } else {
                "  (no graphics queue)"
            },
            if first_match {
                "   <- first with a graphics queue"
            } else {
                ""
            }
        );
    }
    unsafe { instance.destroy_instance(None) };
}
