//! Known Keychron products, by USB product ID.
//!
//! Keychron Launcher downloads its device list; Openchron ships the product IDs observed on
//! real hardware and in community tools (see `docs/keychron-hid.md`). Unknown products still
//! work: they are named after their USB product string.

struct Product {
    id: u16,
    name: &'static str,
    receiver: bool,
}

const PRODUCTS: &[Product] = &[
    Product {
        id: 0xD028,
        name: "Keychron Ultra-Link 8K",
        receiver: true,
    },
    Product {
        id: 0xD030,
        name: "Keychron Link",
        receiver: true,
    },
    Product {
        id: 0xD031,
        name: "Keychron Link",
        receiver: true,
    },
    Product {
        id: 0xD038,
        name: "Keychron 4K Link",
        receiver: true,
    },
    Product {
        id: 0xD05A,
        name: "Keychron TurboLink 8K",
        receiver: true,
    },
    Product {
        id: 0xD048,
        name: "Keychron M5",
        receiver: false,
    },
    Product {
        id: 0xD049,
        name: "Keychron M6 8K",
        receiver: false,
    },
    Product {
        id: 0xD056,
        name: "Keychron M7 8K",
        receiver: false,
    },
    Product {
        id: 0xD060,
        name: "Keychron M6",
        receiver: false,
    },
    Product {
        id: 0xD06E,
        name: "Keychron G3",
        receiver: false,
    },
    Product {
        id: 0xD09D,
        name: "Keychron G6 HE",
        receiver: false,
    },
];

fn find(product_id: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|product| product.id == product_id)
}

/// Whether the product is a 2.4 GHz receiver, which relays the device paired with it.
#[must_use]
pub fn is_receiver(product_id: u16) -> bool {
    find(product_id).is_some_and(|product| product.receiver)
}

/// Display name of a product: its catalog name, else its USB product string, else a name
/// built from `kind` and the product ID.
#[must_use]
pub fn name(product_id: u16, usb_product: Option<&str>, kind: &str) -> String {
    find(product_id).map_or_else(
        || {
            usb_product
                .map(str::trim)
                .filter(|product| !product.is_empty())
                .map_or_else(
                    || format!("Keychron {kind} ({product_id:04X})"),
                    str::to_owned,
                )
        },
        |product| product.name.to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::{is_receiver, name};

    #[test]
    fn receivers_are_recognized() {
        // Given the TurboLink 8K receiver and the G6 HE mouse
        // When checking whether they are receivers
        // Then only the receiver is
        assert!(is_receiver(0xD05A));
        assert!(!is_receiver(0xD09D));
        assert!(!is_receiver(0x0001));
    }

    #[test]
    fn names_fall_back_to_the_usb_product_string() {
        // Given a known product, an unknown one with a product string, and one without
        // When naming them
        // Then the catalog wins, then the product string, then a generic name
        assert_eq!(name(0xD09D, Some("ignored"), "mouse"), "Keychron G6 HE");
        assert_eq!(name(0x0001, Some(" Keychron M3 "), "mouse"), "Keychron M3");
        assert_eq!(name(0x0001, Some(""), "mouse"), "Keychron mouse (0001)");
        assert_eq!(name(0x00AB, None, "keyboard"), "Keychron keyboard (00AB)");
    }
}
