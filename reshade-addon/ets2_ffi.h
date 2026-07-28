// Declaracoes das funcoes/structs `extern "C"` expostas pela staticlib
// Rust (src/ffi.rs, compilada como `ets2_spotify_core.lib` via
// `cargo build --release --lib` e linkada por build.bat).
//
// Espelhado a mao contra src/ffi.rs - mesma disciplina que o projeto ja
// usava pra sincronizar `OverlayShared` entre overlay_ipc.rs e
// overlay_addon.cpp (removido nesta migracao), so que agora sao
// assinaturas de funcao/structs simples em vez de um struct de memoria
// compartilhada entre processos.
#pragma once

#include <cstddef>
#include <cstdint>

namespace ets2
{
	constexpr size_t kTitleCap = 128;
	constexpr size_t kArtistCap = 96;
	constexpr size_t kStatusCap = 160;
	constexpr size_t kPlaylistNameCap = 64;
	constexpr size_t kPlaylistUriCap = 160;
	constexpr size_t kDeviceIdCap = 128;
	constexpr size_t kDeviceNameCap = 96;

#pragma pack(push, 1)
	struct Snapshot
	{
		uint32_t electrics_on;
		uint32_t game_paused;
		uint32_t telemetry_connected;
		uint32_t has_track;
		uint32_t title_len;
		uint32_t artist_len;
		uint8_t title[kTitleCap];
		uint8_t artist[kArtistCap];
		uint64_t thumb_generation;
		uint32_t thumb_width;
		uint32_t thumb_height;
		uint32_t status_len;
		uint8_t status[kStatusCap];
		uint32_t volume;
		uint32_t position_ms;
		uint32_t duration_ms;
	};

	struct Playlist
	{
		uint32_t name_len;
		uint32_t uri_len;
		uint8_t name[kPlaylistNameCap];
		uint8_t uri[kPlaylistUriCap];
	};

	struct OutputDevice
	{
		uint32_t id_len;
		uint32_t name_len;
		uint8_t id[kDeviceIdCap];
		uint8_t name[kDeviceNameCap];
	};
#pragma pack(pop)

	// Comandos aceitos por ets2_send_command - espelha `Command` em
	// src/state.rs / o `match kind` em src/ffi.rs::ets2_send_command.
	enum class CommandKind : uint32_t
	{
		PlayPause = 0,
		Next = 1,
		Previous = 2,
		PlayUri = 3,
		Play = 4,
		Pause = 5,
	};
} // namespace ets2

extern "C"
{
	bool ets2_engine_start();
	void ets2_engine_shutdown();

	bool ets2_poll_snapshot(ets2::Snapshot *out);
	bool ets2_get_thumbnail(uint64_t expect_generation, uint8_t *out_buf, size_t buf_cap, size_t *out_len);
	bool ets2_send_command(uint32_t kind, const uint8_t *text_ptr, size_t text_len);
	bool ets2_set_volume(uint32_t percent);

	bool ets2_list_playlists(ets2::Playlist *out, size_t cap, size_t *out_count);
	bool ets2_add_playlist(const uint8_t *name_ptr, size_t name_len, const uint8_t *uri_ptr, size_t uri_len);
	bool ets2_remove_playlist(size_t index);

	bool ets2_list_output_devices(ets2::OutputDevice *out, size_t cap, size_t *out_count);
	bool ets2_set_output_device(const uint8_t *id_ptr, size_t id_len);
}
