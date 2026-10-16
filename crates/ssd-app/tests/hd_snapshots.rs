//! The twelve reference frames of M6 task 3: the Enhanced picture at 4x, drawn by `--ui-snapshot`
//! from the reference replays (`.work/replays`) and written to `.work/hd/` (untracked).
//!
//! Each picture is compared with a signature, not with its bytes: the GPU rounds the texture
//! filtering differently from one card to another (a channel off by one or two on a fraction of a
//! percent of the pixels), so the bytes of the file differ while the picture does not. The
//! signature is coarse on purpose. Per cell of 128 x 128 pixels it keeps the mean of each channel
//! and the mean edge strength; for the whole picture, the centre of the edge strength. A cell
//! mean is an average of 16384 pixels, and the centre a ratio of sums over the whole picture: the
//! signature cannot be turned back into the picture, and it holds no pixel of the game.
//!
//! A value may move by one bucket (one level of a channel, two levels of edge strength, a quarter
//! of a pixel of centre) and still match. GPU rounding moves a value by a few hundredths of a
//! bucket, so it always matches; a real change moves values by many buckets (a blanked text cell,
//! a recoloured window, a picture shifted by one pixel, a filter) and always fails.
//!
//! Needs `SSD_ROM_PATH` (the ROM) and the replays; run with
//! `cargo test -p ssd-app --release --test hd_snapshots -- --ignored`. The signature checks on
//! synthetic pictures run with the other tests of the crate.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// The frames: the name of the PNG, the replay, the frames played before the picture, and the
/// signature of the picture (see [`encode`]).
const FRAMES: [(&str, &str, u64, &str); 12] = [
    (
        "01-title",
        "landing",
        1600,
        "049203bc0000ff002323b60149498002393993013c3c89023a3a8d022f1ba10152527e010202e200\
        0000fe000000ff001a1ab4016868350379792d0454545003645c2f047c240d03645f36020000fe00\
        0000ff000000ff002e009100a700020188000f018600050114008000a1000d01350072000000ff00\
        0000ff000000ff001111f6023916b304211eda04170ddc0218009c00860023012c007a000000ff00\
        0000ff000000ff002021f6019588c908857bd1079288d407877dcd08988ace082d2af2020000ff00\
        0000ff000000ff00272ffe02b8b7d80899a1ea06919ce905a5abe207b0acd2084149fa040000ff00\
        0000ff000000ff000000ff001818ff043232ff073a3aff072e2eff070909ff010000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff0008f2086b",
    ),
    (
        "02-flight",
        "landing",
        500,
        "049203bc0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000fa00\
        0000fe000000ff000000ff000000ff000000ff000000ff000000ff000f0fff000000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000405ff007686ff020000ff000000ff00\
        0000ff000000ff000000ff000000ff000e10ff010000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff002525ff010000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff00272bff01b7ceff011315ff010000ff000000ff000000ff000000ff000000ff00\
        0000ff000000ff000000ff000001ff000000ff000000ff000000ff000000ff000000ff000000ff00\
        0000ff00081f08fd",
    ),
    (
        "03-field",
        "story-1",
        9500,
        "049203bc66b6fe0064b5fe0066b6fe0065b6fe0066b6fe0067b6fe0065b6fe0066b6fe0064b3f900\
        68b7fd0166b6fe0066b6fe0063b5fe0067b7fe0166b6fe0066b6fe0066b6fe0065b6fe0064b5fe00\
        63b5ff0066b6fe0065b6fe0074bced0170bfa80368b76f056dbd98047abed30271bafa0166b6fe00\
        67b7fe0165b6fe0066b6fe0086bedd0360ac4c065d8b490a4d91430673b94d0583b7ab0466b6fe00\
        68b7fe0164b5fe0065b6fe006ab8fe0175ba7105409a330666ab430687ae920573b7f10164b5fe00\
        64b5ff0066b6fe0066b6fe006ab8fe018cbc9d0572ab470681af960569b8fe0165b6fe0066b6fe00\
        68b7fe0166b6fe0064b5fe0066b6fe006bb8fe0174b9f5016db9fd0165b6fe0064b5fe0066b6fe00\
        68b7fe0166b6fe0064b5fe0064b5fe0067b7fe0164b5fe0066b6fe0063b5ff0066b6fe0064b5fe00\
        63b5ff00098d07be",
    ),
    (
        "04-message",
        "landing",
        2400,
        "049203bc0000840003006d000600270003000300000000000300030007001e000300640000008200\
        000083000000840003006d000800290003000300000000000300030007001e000600640000008400\
        000084000000840003006d0009002a0002000200060301000400030004001d000700640000008400\
        000084000000840005006e000600280000000000270f05030100010003001d000600650000008400\
        0000840006068601050270000b062c01050505010e070501080808010a0622010b06690107078701\
        060687013f3fa20934308a083b36540934343409222222063a3a3a0a2c284106383585091c1c9104\
        2222940531319c074441930a2d294906353535082f2f2f083b3a3b0928223c051817740416168f03\
        13138d032d2d99063c398f082c294706262626074545450a2623260737324a083e3c880905058600\
        00008400086d0b9b",
    ),
    (
        "05-command-menu",
        "story-1",
        9900,
        "049203bc5f99cd035f89b00566b6fe0066b6fe0064b5fe0065b6fe0064b5fe0068a9d006609fc406\
        63b4fe006fa3d8037497bd0464b5fe0066b6fe0065b6fe0066b6fe0065b6fe0068abd30562a6d105\
        63b5fe0065b6fe0065b6fe0065b6fe0065b6fe006ab8fe016bb8fe0166b6fe0064b5fe0065b6fe00\
        63b5ff0066b6fe0066b6fe006cb8fd0077bfbd03608f5a085eb95e0477beb6037bbeda0268b7fe01\
        63b5fe0081acdc02a4b1c404a1acc2048dbe970542a5370653855c0869b64a066aab560679bffd01\
        63b5fe009cacc602e6d6bd06dbc6b40288bec80446aa37064aae39068aaa730683b8d70364b5fe00\
        63b5ff009fb3cc02e3d4bc06d7c2b2037fbce70284b46e067dac5c0678baf30165b6fe0067b6fe00\
        63b5fe007ba2d4029d9cb1039197b30366b6fe006fbafd0176b9f60265b6fe0066b6fe0064b5fe00\
        63b5ff000900082b",
    ),
    (
        "06-menu-list",
        "secret-namek-zfighters",
        2700,
        "049203bc294b570534464b053b5144041845590303436d030a4569031144610350615e0537566804\
        1e4a5803795d2b099070460b5a3e0405343623034a4d2b055f4104044b421a056549250962471e08\
        5b4004026c572e09766040093e2502044128020453380406684a05043e2501046f4f16073a1e0b06\
        06000000422e1d084e3b2e07452e0304432801043922070538200205563c04053820010471510504\
        4f3200035439260582624f0695684a07bd927307b8977f0691746104342a140527180704321e0604\
        0b101302a5865f04e6d8bf05edba9703e6d4ba05e2d4bc07ab9f940202447003164965032a4c5604\
        5e5f4404636b7504e2d3ba07e8b79604e6d3ba06efe0c404b2a2920206456c030144700306497403\
        125b83022a466203675f64036453580364585f03675f6503474e5f033f5450041c4b6103265e7503\
        1f7199010875070b",
    ),
    (
        "07-menu-status",
        "secret-namek-zfighters",
        3000,
        "049203bc2f1c1502755c4d056a4f40052d241d05745b4c0567554804634a3d066955430530221603\
        0000000061473803ebdcc204d8c9b209ceb99b09dbcdb809d7bd9b08cfbda30ad8c4a1086f543a04\
        00000000583f3305d5c7b40abfb3a30d93837008cdc1af0caca69f0cc3b8aa0891887b083b2e2601\
        0000000061473803ebdcc104d2c3ad09c5a88909d9ccb709c4b5ae09e3d6c302baae960048392e00\
        00000000583f3305d9cbb609c8bba8096c655904d4c7b30a7a726605d0c3aa02aea38d013f322900\
        0000000061473803f4e5c702eddec000857d6c02ffefce00988e7b01dacdb101baae960048392e00\
        000000003d251d03a1857104af937a00ad917901af937a00af937a00ad917801af937a005d413301\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000008440582",
    ),
    (
        "08-battle-command",
        "story-1",
        8400,
        "049203bc5e97db01a3b3ca0391a5c3027690b3045676990796abc9038fa8c90292abca02789ed002\
        2d96fe0089a1c702e4d4bc06eccdae04c8ae9709af867b0ce6d7be05e1d3bb07faeaca0199abc501\
        2994ff005e99df028ca9ce0198aac80282a0c90181a2cb018fa5c6047d9ec903a4b5ce026297d801\
        2994ff002994ff003a9dff012994ff002994ff003698fa0149a4ff013198ff002994ff0042a0ff01\
        2d96ff00529fe302639bd7035b98d7033cabfe013f89c20b62b2f60354b5ff023cabff015fafe502\
        69a3ce019e857104e9c7a905d3ad96035f7f95043f75a2035a7f98058a977f06889c830586916e03\
        798a6e0175606403e5d5bc06ccae99034e464107564b42076e544f0870512c0b7155320c574b4207\
        2131520042374a017854540168474c013a415304464650034a4a510653515507524f530743455004\
        21315200096d0849",
    ),
    (
        "09-attack",
        "story-1",
        8900,
        "049203bcf8730005f27d0006f67e0005f5890004fa8f0003f6870004f7800005f17d0006f3730005\
        fa700003f88a0003f97e0005f4720009f071000af27e0006f0720009f3700009f97c0005f8890003\
        f8870003f4850004f4850005f27c0007e762010dab630c0cec68000bf27a0007f4840005f4840004\
        f2870003f4840004f3860005f27c0007ea62000dd076050ade62030df2790007f3850005f4850004\
        f2800004f88a0003f97c0005f3720009f1740009f37f0005f1750008f2710009f97a0005f8880003\
        f89000025a3f36038d553308e6b07e06eab58005eab58005e6b27e06edb78205a47b610868595705\
        5140400037384e027e585106ffefce00ffefce00ffefce00ffefce00ffefce007c727a0521375b03\
        213152002131520036324900785454017854540178545401785454017854540143374a0021315200\
        2131520009170684",
    ),
    (
        "10-ending-story",
        "secret-ending-normal",
        6000,
        "049203bc0804020028270802302b0c022d2a0b02292a0a02332b0d02282a0902282a09020f070300\
        00000000200e07016ce6230272eb240274d41d0457ba220560e81e0162ee200158ee1f012d270a01\
        00000000200e07019bdda703b7d89c03a58b550a95a48d0798c4b903a0e5ad02a5cfaa033a261a01\
        00000000200e070185bee602c1c1b503c0b4a2039cb7c3028bc3e5018dc5e80189b9d5023a221801\
        000000001f0e07015783b5065d8cbc057995b105658eb8065188c0064d87c2065188c1062c1b1901\
        000000000000000018181806151515051b1b1b071a1a1a070f0f0f040f0f0f040303030000000000\
        00000000000000000c0c0c030e0e0e030d0d0d030e0e0e03090909020d0d0d040101010000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000878077f",
    ),
    (
        "11-ending-credits",
        "secret-ending-normal",
        9000,
        "049203bc080402002b270a022d290902282a0902282a0902332b0d02292a0a022d2a0c0210070300\
        00000000200e070165e6200162ec220163d7190464be250570e823025fee200165ee23012f270a01\
        00000000200e0701acd07403b1cb90038492770b8cb59707b2e0ac0292da9c03addfbe0339231801\
        00000000200e0701c7b6a004a4b0b10287c5ec0190c3e202a2b6bd0378b8e60291c4e2013b221801\
        000000001f0e07014d80ba05678eb706578abd065d8cbb068498ac055d8cbb064a86c3062c1b1901\
        0000000000000000091000020b1300021314110331313108151514030b1300020b13000200000000\
        00000000000000000000000013181f02232e3a041b232d03414c5908293644050304050000000000\
        000000000000000000000000000000000000000000000000000000000f1415020304050000000000\
        0000000008e5076d",
    ),
    (
        "12-ending-end",
        "secret-ending-normal",
        15000,
        "049203bc000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000a0a0a004747470047474700323232010c0c0c0033333301474747013535350102020200\
        000000000f1e210066c1d901538b9a02696d580890754c0a7a795c0963bbd1004b737d012d677500\
        0000000014181900899ea3017f7f7f016f6a6603473f38036e665d0392a09c0275858a021a2a2e00\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000092a076a",
    ),
];

/// The side of a cell of the signature, in pixels of the picture: 128 pixels, 32 game pixels at 4x.
const CELL: usize = 128;
/// The edge strength of a cell is kept in steps of this many levels, so that it fits a byte.
const EDGE_STEP: u64 = 2;
/// The smallest luma step that counts as an edge. GPU rounding moves a channel by one or two
/// levels, so the steps of a flat area move by five levels at most: below eight they are noise.
const EDGE_FLOOR: u32 = 8;
/// The centre of the edge strength is kept in steps of a quarter of a pixel.
const CENTRE_STEPS_PER_PIXEL: u64 = 4;
/// How far a value may move, in buckets, and still match.
const TOLERANCE: u16 = 1;

/// The signature of one picture: its size, per cell (row by row) the mean of red, green and blue
/// and the mean edge strength, and the centre of the edge strength over the whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
    width: u32,
    height: u32,
    cells: Vec<[u8; 4]>,
    centre: [u16; 2],
}

/// The luma of an RGBA pixel, in levels (0 to 255).
fn luma(pixel: &[u8]) -> u32 {
    (77 * u32::from(pixel[0]) + 150 * u32::from(pixel[1]) + 29 * u32::from(pixel[2])) >> 8
}

/// The luma step between two neighbouring pixels, if it is an edge: at least `EDGE_FLOOR` levels.
fn step(a: u32, b: u32) -> u32 {
    let difference = a.abs_diff(b);
    if difference >= EDGE_FLOOR { difference } else { 0 }
}

/// The signature of `rgba` (`width` x `height` pixels, top row first). The edge strength of a
/// pixel is the step to its right plus the step below it (see [`step`]); the last column and row
/// have none.
fn signature(width: u32, height: u32, rgba: &[u8]) -> Signature {
    let (w, h) = (width as usize, height as usize);
    let luma: Vec<u32> = rgba.as_chunks::<4>().0.iter().map(|p| luma(p)).collect();
    let mut edge = vec![0_u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let right = if x + 1 < w { step(luma[i], luma[i + 1]) } else { 0 };
            let below = if y + 1 < h { step(luma[i], luma[i + w]) } else { 0 };
            edge[i] = right + below;
        }
    }
    let mut cells = Vec::new();
    for y0 in (0..h).step_by(CELL) {
        for x0 in (0..w).step_by(CELL) {
            let (x1, y1) = ((x0 + CELL).min(w), (y0 + CELL).min(h));
            let mut sums = [0_u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * w + x;
                    for (sum, &channel) in sums.iter_mut().zip(&rgba[4 * i..4 * i + 3]) {
                        *sum += u64::from(channel);
                    }
                    sums[3] += u64::from(edge[i]);
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            cells.push([
                bucket(sums[0], count, 1),
                bucket(sums[1], count, 1),
                bucket(sums[2], count, 1),
                bucket(sums[3], count, EDGE_STEP),
            ]);
        }
    }
    let (mut total, mut moment_x, mut moment_y) = (0_u64, 0_u64, 0_u64);
    for (i, &strength) in edge.iter().enumerate() {
        let strength = u64::from(strength);
        total += strength;
        moment_x += strength * (i % w) as u64;
        moment_y += strength * (i / w) as u64;
    }
    let centre = if total == 0 {
        [u16::MAX; 2]
    } else {
        [moment_x, moment_y].map(|moment| {
            u16::try_from(CENTRE_STEPS_PER_PIXEL * moment / total).unwrap_or(u16::MAX)
        })
    };
    Signature { width, height, cells, centre }
}

/// `sum / count` in steps of `step` levels, saturated to a byte.
fn bucket(sum: u64, count: u64, step: u64) -> u8 {
    u8::try_from(sum / (count * step)).unwrap_or(u8::MAX)
}

/// The values of `actual` that are beyond [`TOLERANCE`] from `expected`, as messages (empty when
/// the pictures match). A picture of another size is one message.
fn differences(actual: &Signature, expected: &Signature) -> Vec<String> {
    if (actual.width, actual.height) != (expected.width, expected.height) {
        return vec![format!(
            "the picture is {}x{}, expected {}x{}",
            actual.width, actual.height, expected.width, expected.height
        )];
    }
    let cols = (expected.width as usize).div_ceil(CELL);
    let names = ["red", "green", "blue", "edge"];
    let mut out = Vec::new();
    for (i, (a, e)) in actual.cells.iter().zip(&expected.cells).enumerate() {
        for k in 0..4 {
            if u16::from(a[k].abs_diff(e[k])) > TOLERANCE {
                out.push(format!(
                    "cell {},{} {}: {} expected {}",
                    i % cols,
                    i / cols,
                    names[k],
                    a[k],
                    e[k]
                ));
            }
        }
    }
    for (k, axis) in ["x", "y"].iter().enumerate() {
        if actual.centre[k].abs_diff(expected.centre[k]) > TOLERANCE {
            out.push(format!(
                "the centre of the edges {axis}: {} expected {} (quarter pixels)",
                actual.centre[k], expected.centre[k]
            ));
        }
    }
    out
}

/// Whether the pictures differ beyond the tolerance.
fn differs(actual: &Signature, expected: &Signature) -> bool {
    !differences(actual, expected).is_empty()
}

/// The signature as hex text: the size (two 16-bit numbers), then per cell its four bytes, then
/// the centre (two 16-bit numbers), all big-endian. The entries of [`FRAMES`] are written this way.
fn encode(signature: &Signature) -> String {
    let mut bytes = Vec::new();
    bytes.extend(u16::try_from(signature.width).unwrap().to_be_bytes());
    bytes.extend(u16::try_from(signature.height).unwrap().to_be_bytes());
    for cell in &signature.cells {
        bytes.extend(cell);
    }
    for value in signature.centre {
        bytes.extend(value.to_be_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().concat()
}

/// The signature that [`encode`] wrote, read back.
fn decode(text: &str) -> Signature {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect();
    let width = u32::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    let height = u32::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    let cells_count = (width as usize).div_ceil(CELL) * (height as usize).div_ceil(CELL);
    let body = &bytes[4..4 + 4 * cells_count];
    let cells = body.as_chunks::<4>().0.to_vec();
    let tail = &bytes[4 + 4 * cells_count..];
    let centre = [u16::from_be_bytes([tail[0], tail[1]]), u16::from_be_bytes([tail[2], tail[3]])];
    Signature { width, height, cells, centre }
}

/// The largest difference of the signatures, in buckets (zero for pictures that agree).
fn deviation(actual: &Signature, expected: &Signature) -> u16 {
    if (actual.width, actual.height) != (expected.width, expected.height) {
        return u16::MAX;
    }
    let cells = actual
        .cells
        .iter()
        .zip(&expected.cells)
        .flat_map(|(a, e)| (0..4).map(|k| u16::from(a[k].abs_diff(e[k]))));
    let centre = (0..2).map(|k| actual.centre[k].abs_diff(expected.centre[k]));
    cells.chain(centre).max().unwrap_or(0)
}

/// Deterministic pseudo-random numbers (xorshift64): every run draws the same noise.
struct Draws(u64);

impl Draws {
    fn draw(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `rgba` with a colour channel off by one or two on `per_mille` pixels in a thousand: what a
/// different GPU's rounding does to the picture.
fn rounded_differently(rgba: &[u8], seed: u64, per_mille: u64) -> Vec<u8> {
    let mut draws = Draws(seed);
    let mut out = rgba.to_vec();
    for pixel in out.as_chunks_mut::<4>().0 {
        if draws.draw() % 1000 < per_mille {
            for channel in &mut pixel[..3] {
                let step = i8::try_from(1 + draws.draw() % 2).unwrap_or(1);
                let sign: i8 = if draws.draw().is_multiple_of(2) { 1 } else { -1 };
                *channel = channel.saturating_add_signed(sign * step);
            }
        }
    }
    out
}

/// `rgba` moved one pixel to the right, the first column repeated.
fn shifted_right(rgba: &[u8], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for row in rgba.chunks_exact(4 * width) {
        out.extend_from_slice(&row[..4]);
        out.extend_from_slice(&row[..row.len() - 4]);
    }
    out
}

/// `rgba` with the square of `side` pixels at (`x0`, `y0`) painted in its most common colour.
fn blanked(rgba: &[u8], width: usize, (x0, y0): (usize, usize), side: usize) -> Vec<u8> {
    let mut colours: HashMap<[u8; 4], usize> = HashMap::new();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let i = 4 * (y * width + x);
            *colours.entry(rgba[i..i + 4].try_into().unwrap()).or_default() += 1;
        }
    }
    let background = colours.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| c).unwrap_or([0; 4]);
    let mut out = rgba.to_vec();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let i = 4 * (y * width + x);
            out[i..i + 4].copy_from_slice(&background);
        }
    }
    out
}

/// `rgba` with 24 levels more red in the rectangle from (`x0`, `y0`) to (`x1`, `y1`).
fn recoloured(
    rgba: &[u8],
    width: usize,
    (x0, y0): (usize, usize),
    (x1, y1): (usize, usize),
) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for y in y0..y1 {
        for x in x0..x1 {
            let i = 4 * (y * width + x);
            out[i] = out[i].saturating_add(24);
        }
    }
    out
}

/// `rgba` with every colour channel averaged over the 3 by 3 pixels around it.
fn blurred(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                let (mut sum, mut count) = (0_u32, 0_u32);
                for ny in y.saturating_sub(1)..(y + 2).min(height) {
                    for nx in x.saturating_sub(1)..(x + 2).min(width) {
                        sum += u32::from(rgba[4 * (ny * width + nx) + channel]);
                        count += 1;
                    }
                }
                out[4 * (y * width + x) + channel] = u8::try_from(sum / count).unwrap_or(u8::MAX);
            }
        }
    }
    out
}

/// The 32 x 32 square (at a multiple of 32) with the largest luma change: a text cell of the picture.
fn busiest_square(rgba: &[u8], width: usize, height: usize) -> (usize, usize) {
    let mut best = ((0, 0), 0);
    for y0 in (0..height.saturating_sub(32)).step_by(32) {
        for x0 in (0..width.saturating_sub(32)).step_by(32) {
            let mut edges = 0;
            for y in y0..y0 + 32 {
                for x in x0..x0 + 31 {
                    let i = 4 * (y * width + x);
                    edges += luma(&rgba[i..i + 4]).abs_diff(luma(&rgba[i + 4..i + 8]));
                }
            }
            if edges > best.1 {
                best = ((x0, y0), edges);
            }
        }
    }
    best.0
}

/// The checks of one rendered picture: the rounding of a GPU must match, and each real change
/// must not. Returns what went wrong.
fn check_rendered(
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    expected: &Signature,
) -> Vec<String> {
    let (w, h) = (width as usize, height as usize);
    let mut problems = Vec::new();
    let actual = signature(width, height, rgba);
    if deviation(&actual, expected) > TOLERANCE {
        problems.push(format!("{name}: {}", differences(&actual, expected).join("; ")));
    }
    let changes = [
        ("1% of the pixels off by one or two", rounded_differently(rgba, 7, 10), true),
        ("shifted by one pixel", shifted_right(rgba, w), false),
        ("a text cell blanked", blanked(rgba, w, busiest_square(rgba, w, h), 32), false),
        ("a window recoloured", recoloured(rgba, w, (w / 4, h / 4), (w / 2, h / 2)), false),
        ("a filter applied", blurred(rgba, w, h), false),
    ];
    for (what, changed, must_match) in changes {
        let differences = differences(&signature(width, height, &changed), expected);
        if differences.is_empty() != must_match {
            problems.push(format!(
                "{name}: {what} gives {}",
                if differences.is_empty() { "a match" } else { "a mismatch" }
            ));
        }
    }
    problems
}

#[test]
#[ignore = "needs SSD_ROM_PATH and the reference replays"]
fn the_twelve_reference_frames_at_four_times_match_their_signatures() {
    let rom = std::env::var("SSD_ROM_PATH").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let replays = root.join(".work").join("replays");
    let out_dir = root.join(".work").join("hd");
    std::fs::create_dir_all(&out_dir).unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut problems = Vec::new();
    for (name, replay, frames, reference) in FRAMES {
        let out = out_dir.join(format!("{name}.png"));
        let run = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
            .args(["--rom", &rom, "--replay"])
            .arg(replays.join(format!("{replay}.ssdreplay")))
            .args(["--headless-frames", &frames.to_string()])
            .args(["--hd-scale", "4", "--no-hd-font", "--ui-size", "1170x956", "--ui-snapshot"])
            .arg(&out)
            .env("SSD_DATA_DIR", data.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(0), "{name}: stderr: {stderr}");
        let (width, height, rgba) = ssd_hd::png::read_rgba(&std::fs::read(&out).unwrap()).unwrap();
        let expected = decode(reference);
        let actual = signature(width, height, &rgba);
        let differences = differences(&actual, &expected);
        if !differences.is_empty() {
            problems.push(format!(
                "{name}: {}\n  its signature is {}",
                differences.join("; "),
                encode(&actual)
            ));
        }
        problems.extend(check_rendered(name, width, height, &rgba, &expected));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A stand-in for a frame: a dark ground with a gradient, blocks of 4 pixels in a few colours,
/// glyph-like cells of 32 pixels in two colours, and a window with a cream border.
fn synthetic_picture(width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let (bx, by) = (x / 4, y / 4);
            let hash = (bx * 73 + by * 151 + (bx * by) % 17) % 97;
            let mut pixel = [20 + (x / 8 % 16) as u8, 30, 60, 255];
            if (100..500).contains(&x) && (200..400).contains(&y) {
                pixel = if (104..496).contains(&x) && (204..396).contains(&y) {
                    [40, 40, 90, 255]
                } else {
                    [230, 220, 180, 255]
                };
            } else if (x / 32 + y / 32) % 3 == 0 && hash % 2 == 0 {
                let glyph = ((x / 4) % 8 + (y / 4) % 8 * 3 + hash) % 5 < 2;
                pixel = if glyph { [240, 230, 200, 255] } else { [10, 10, 40, 255] };
            }
            out.extend_from_slice(&pixel);
        }
    }
    out
}

/// The synthetic picture at the size of the snapshots, and its signature.
fn synthetic_frame() -> (Vec<u8>, Signature) {
    let (width, height) = (1170, 956);
    let picture = synthetic_picture(width, height);
    let expected = signature(width as u32, height as u32, &picture);
    (picture, expected)
}

#[test]
fn gpu_rounding_on_one_percent_of_the_pixels_matches() {
    let (picture, expected) = synthetic_frame();
    let rounded = rounded_differently(&picture, 7, 10);
    let actual = signature(expected.width, expected.height, &rounded);
    assert!(differences(&actual, &expected).is_empty(), "{:?}", differences(&actual, &expected));
}

#[test]
fn a_picture_shifted_by_one_pixel_fails() {
    let (picture, expected) = synthetic_frame();
    let shifted = shifted_right(&picture, expected.width as usize);
    let actual = signature(expected.width, expected.height, &shifted);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_blanked_text_cell_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let at = busiest_square(&picture, w, h);
    let blank = blanked(&picture, w, at, 32);
    let actual = signature(expected.width, expected.height, &blank);
    assert!(!differences(&actual, &expected).is_empty(), "the cell at {at:?}");
}

#[test]
fn a_recoloured_window_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let recolour = recoloured(&picture, w, (w / 4, h / 4), (w / 2, h / 2));
    let actual = signature(expected.width, expected.height, &recolour);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_filtered_picture_fails() {
    let (picture, expected) = synthetic_frame();
    let (w, h) = (expected.width as usize, expected.height as usize);
    let filtered = blurred(&picture, w, h);
    let actual = signature(expected.width, expected.height, &filtered);
    assert!(differs(&actual, &expected));
}

#[test]
fn a_picture_of_another_size_fails() {
    let (picture, expected) = synthetic_frame();
    let smaller = signature(expected.width - 1, expected.height, &picture[..4 * 1169 * 956]);
    assert!(differs(&smaller, &expected));
}
