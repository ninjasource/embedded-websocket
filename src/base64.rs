// *************************************** BASE64 ENCODE ******************************************
// The base64_encode function below was adapted from the rust-base64 library
// https://github.com/alicemaz/rust-base64
// The MIT License (MIT)
// Copyright (c) 2015 Alice Maz, 2019 David Haig
// Adapted for no_std specifically for MIME (Standard) flavoured base64 encoding
// ************************************************************************************************

pub const BASE64_ENCODE_TABLE: &'static [u8; 64] = &[
    65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88,
    89, 90, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114,
    115, 116, 117, 118, 119, 120, 121, 122, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 43, 47,
];

pub fn encode(input: &[u8], output: &mut [u8]) -> usize {
    let encode_table: &[u8; 64] = BASE64_ENCODE_TABLE;
    let mut input_index: usize = 0;
    let mut output_index = 0;
    const LOW_SIX_BITS_U8: u8 = 0x3F;
    let rem = input.len() % 3;
    let start_of_rem = input.len() - rem;

    while input_index < start_of_rem {
        let input_chunk = &input[input_index..(input_index + 3)];
        let output_chunk = &mut output[output_index..(output_index + 4)];

        output_chunk[0] = encode_table[(input_chunk[0] >> 2) as usize];
        output_chunk[1] =
            encode_table[((input_chunk[0] << 4 | input_chunk[1] >> 4) & LOW_SIX_BITS_U8) as usize];
        output_chunk[2] =
            encode_table[((input_chunk[1] << 2 | input_chunk[2] >> 6) & LOW_SIX_BITS_U8) as usize];
        output_chunk[3] = encode_table[(input_chunk[2] & LOW_SIX_BITS_U8) as usize];

        input_index += 3;
        output_index += 4;
    }

    if rem == 2 {
        output[output_index] = encode_table[(input[start_of_rem] >> 2) as usize];
        output[output_index + 1] = encode_table[((input[start_of_rem] << 4
            | input[start_of_rem + 1] >> 4)
            & LOW_SIX_BITS_U8) as usize];
        output[output_index + 2] =
            encode_table[((input[start_of_rem + 1] << 2) & LOW_SIX_BITS_U8) as usize];
        output_index += 3;
    } else if rem == 1 {
        output[output_index] = encode_table[(input[start_of_rem] >> 2) as usize];
        output[output_index + 1] =
            encode_table[((input[start_of_rem] << 4) & LOW_SIX_BITS_U8) as usize];
        output_index += 2;
    }

    // add padding
    let rem = input.len() % 3;
    for _ in 0..((3 - rem) % 3) {
        output[output_index] = b'=';
        output_index += 1;
    }

    output_index
}

// ************************************************************************************************
// **************************************** TESTS *************************************************
// ************************************************************************************************

#[cfg(test)]
mod tests {
    extern crate std;

    // ASCII values A-Za-z0-9+/
    pub const STANDARD_ENCODE: &'static [u8; 64] = &[
        65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87,
        88, 89, 90, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112,
        113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
        43, 47,
    ];

    #[test]
    fn base64_encode_test() {
        let input: &[u8] = &[0; 20];
        let output: &mut [u8] = &mut [0; 100];
        let encode_table: &[u8; 64] = STANDARD_ENCODE;
        let mut input_index: usize = 0;
        let mut output_index = 0;

        const LOW_SIX_BITS_U8: u8 = 0x3F;

        let rem = input.len() % 3;
        let start_of_rem = input.len() - rem;

        while input_index < start_of_rem {
            let input_chunk = &input[input_index..(input_index + 3)];
            let output_chunk = &mut output[output_index..(output_index + 4)];

            output_chunk[0] = encode_table[(input_chunk[0] >> 2) as usize];
            output_chunk[1] = encode_table
                [((input_chunk[0] << 4 | input_chunk[1] >> 4) & LOW_SIX_BITS_U8) as usize];
            output_chunk[2] = encode_table
                [((input_chunk[1] << 2 | input_chunk[2] >> 6) & LOW_SIX_BITS_U8) as usize];
            output_chunk[3] = encode_table[(input_chunk[2] & LOW_SIX_BITS_U8) as usize];

            input_index += 3;
            output_index += 4;
        }
    }
}