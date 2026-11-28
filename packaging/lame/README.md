# MP3 export and rebuilding the encoder

recordbox's MP3 exporter links the LAME encoder statically through the `mp3lame-encoder` and `mp3lame-sys` Rust crates. The two `.crate` files in this directory are the original crates.io source archives. `mp3lame-sys` includes the LAME C implementation it compiles. Each archive contains the authors' copyright notices. The Rust bindings are licensed under the GNU LGPL version 3. The C library includes its own GNU Library GPL license and notices. [LAME project](https://lame.sourceforge.io/)

The packaged `recordbox-source.txt` links to the complete application source for your binary. The source includes the build and packaging recipes and the locked dependency versions. You can modify the encoder, rebuild recordbox with your version, and run the resulting executable.

1. Download and extract the linked recordbox source archive. Install its pinned Rust toolchain and your platform's build prerequisites from `docs/BUILD-AND-PACKAGING.md`.
2. From the source checkout, extract the shipped codec archives into a new local directory.

   ```sh
   mkdir local-codec
   tar -xzf packaging/lame/mp3lame-sys-0.2.0.crate -C local-codec
   tar -xzf packaging/lame/mp3lame-encoder-0.3.0.crate -C local-codec
   ```

3. Make your changes in `local-codec`. Add the following to `app/Cargo.toml` to compile and relink against these copies.

   ```toml
   [patch.crates-io]
   mp3lame-sys = { path = "../local-codec/mp3lame-sys-0.2.0" }
   mp3lame-encoder = { path = "../local-codec/mp3lame-encoder-0.3.0" }
   ```

4. Run `cargo build --manifest-path app/Cargo.toml --release`. Cargo records the local source paths in your working copy's lockfile. Run the resulting `app/target/release/recordbox` (`recordbox.exe` on Windows). For DirectML, Vulkan or CUDA, follow the corresponding recipe under `docs` and use your modified sources. CUDA's `packaging/build_cuda12.py` normally uses `--locked`, so update the lockfile with your local paths first.
