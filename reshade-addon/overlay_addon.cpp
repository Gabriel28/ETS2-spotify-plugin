// Addon do ReShade que roda dentro do proprio jogo (ETS2/ATS) e da controle
// completo do Spotify sem precisar de nenhum app separado: HUD compacto com
// capa/faixa/artista sempre visivel, um painel expandido (tecla de atalho -
// ver kTogglePanelKey) com botoes de play/pause/anterior/proxima e
// playlists salvas, e hotkeys globais dentro do jogo (ver
// kHotkeyNext/Previous/Play/Pause) que funcionam mesmo com o painel
// fechado.
//
// NAO ha busca: o SMTC (API usada aqui pra falar com o Spotify sem OAuth)
// nao expoe "buscar no catalogo", so controla a sessao ja ativa. Pra tocar
// algo especifico escolhido na hora, abra o Spotify normalmente por fora
// do jogo e escolha la - o play/pause/anterior/proxima daqui continuam
// controlando o que estiver tocando. Playlists salvas (edite
// `%APPDATA%\ets2-spotify-bridge\config.json` - ver config.example.json na
// raiz do projeto) cobrem o caso de reabrir algo sem sair do jogo.
//
// Toda a integracao real (SMTC do Spotify, telemetria do jogo, dispositivo
// de audio, config) vive numa staticlib Rust (../src/*.rs) linkada nesta
// mesma DLL - ver ets2_ffi.h pras assinaturas expostas e build.bat pra como
// a staticlib e compilada/linkada. Este arquivo so cuida do que so o C++
// pode fazer aqui: registro no ReShade e os widgets do ImGui.
//
// Como instalar: compile com build.bat (compila a staticlib Rust primeiro),
// troque a extensao do .dll gerado pra ".addon" e coloque na pasta de
// addons do ReShade (a mesma pasta do ReShade dentro da instalacao do
// jogo, por padrao). Precisa do ReShade ja instalado no ETS2/ATS - ver
// README.md da raiz do projeto.

#include <imgui.h>
#include <reshade.hpp>

#include <algorithm>
#include <cfloat>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>
#include <Windows.h>

#include "ets2_ffi.h"

namespace {

// Tecla que abre/fecha o painel expandido (botoes + playlists). O HUD
// compacto (capa + faixa + artista) fica sempre visivel independente
// disso. Mude aqui se quiser outra tecla.
constexpr uint32_t kTogglePanelKey = VK_F9;

// Hotkeys globais dentro do jogo - funcionam com o painel aberto OU
// fechado (bastando o jogo estar em foco), sem precisar do antigo
// `global-hotkey`/RegisterHotKey: `effect_runtime::is_key_pressed` ja
// reflete o input que o proprio ReShade observa. Cada acao usa uma tecla
// dedicada (em vez de um toggle so) de proposito - assim apertar a tecla
// errada nunca deixa o player num estado que so da pra perceber olhando
// o painel.
constexpr uint32_t kHotkeyNext = VK_PRIOR;    // Ctrl+PageUp: avancar (proxima faixa)
constexpr uint32_t kHotkeyPrevious = VK_NEXT; // Ctrl+PageDown: voltar (faixa anterior)
constexpr uint32_t kHotkeyPlay = VK_INSERT;   // Ctrl+Insert: play
constexpr uint32_t kHotkeyPause = VK_DELETE;  // Ctrl+Delete: pause/stop

// Quantas playlists salvas o painel mostra de uma vez - so limita quantas
// aparecem na lista, nao quantas podem ser salvas (ver ets2_ffi.h).
constexpr size_t kMaxPlaylistsShown = 16;

bool g_engine_started = false;
bool g_panel_expanded = false;

// Textura da capa do album atual, cacheada por geracao (ver
// FfiSnapshot::thumb_generation em src/ffi.rs) - so recriada quando a
// faixa muda, nao a cada frame.
reshade::api::resource g_art_resource = {};
reshade::api::resource_view g_art_view = {};
uint64_t g_art_generation = 0;

void destroy_album_art(reshade::api::device *device)
{
	if (device == nullptr)
		return;
	if (g_art_view.handle != 0)
	{
		device->destroy_resource_view(g_art_view);
		g_art_view = {};
	}
	if (g_art_resource.handle != 0)
	{
		device->destroy_resource(g_art_resource);
		g_art_resource = {};
	}
}

// Cria (ou recria, destruindo a anterior) a textura da capa do album a
// partir dos pixels RGBA que a staticlib Rust decodificou via SMTC. So
// mexe na GPU quando `gen` muda - chamado uma vez por frame, mas o corpo
// so faz trabalho de verdade quando a faixa troca.
void ensure_album_art_texture(reshade::api::device *device, uint32_t width, uint32_t height, uint64_t gen)
{
	if (device == nullptr || gen == g_art_generation || gen == 0 || width == 0 || height == 0)
		return;

	std::vector<uint8_t> pixels(static_cast<size_t>(width) * height * 4);
	size_t written = 0;
	if (!ets2_get_thumbnail(gen, pixels.data(), pixels.size(), &written) || written != pixels.size())
		return; // faixa trocou de novo entre o poll do snapshot e esta chamada - tenta de novo no proximo frame

	destroy_album_art(device);

	const reshade::api::resource_desc desc(
		width, height, 1, 1, reshade::api::format::r8g8b8a8_unorm, 1,
		reshade::api::memory_heap::gpu_only, reshade::api::resource_usage::shader_resource);
	reshade::api::subresource_data initial_data = {};
	initial_data.data = pixels.data();
	initial_data.row_pitch = width * 4;

	if (!device->create_resource(desc, &initial_data, reshade::api::resource_usage::shader_resource, &g_art_resource))
		return;
	if (!device->create_resource_view(
			g_art_resource, reshade::api::resource_usage::shader_resource,
			reshade::api::resource_view_desc(reshade::api::format::r8g8b8a8_unorm), &g_art_view))
	{
		device->destroy_resource(g_art_resource);
		g_art_resource = {};
		return;
	}
	g_art_generation = gen;
}

void draw_now_playing(const ets2::Snapshot &snap)
{
	ImGui::BeginGroup();
	if (g_art_view.handle != 0)
	{
		ImGui::Image(reinterpret_cast<void *>(static_cast<uintptr_t>(g_art_view.handle)), ImVec2(64, 64));
	}
	else
	{
		const ImVec2 p = ImGui::GetCursorScreenPos();
		ImGui::GetWindowDrawList()->AddRectFilled(p, ImVec2(p.x + 64, p.y + 64), IM_COL32(40, 40, 40, 255), 6.0f);
		ImGui::Dummy(ImVec2(64, 64));
	}
	ImGui::EndGroup();

	ImGui::SameLine();
	ImGui::BeginGroup();
	if (snap.has_track)
	{
		std::string title(reinterpret_cast<const char *>(snap.title), snap.title_len);
		std::string artist(reinterpret_cast<const char *>(snap.artist), snap.artist_len);
		ImGui::TextUnformatted(title.empty() ? "(sem titulo)" : title.c_str());
		ImGui::TextColored(ImVec4(0.75f, 0.75f, 0.75f, 1.0f), "%s", artist.c_str());
	}
	else if (snap.status_len > 0)
	{
		// Backend Connect (ver config::Backend) usa esta linha pra avisar
		// de login pendente/reconectando/erro antes de ter uma faixa - ver
		// ffi.rs::ets2_poll_snapshot e spotify_connect.rs::set_status.
		std::string status(reinterpret_cast<const char *>(snap.status), snap.status_len);
		ImGui::TextColored(ImVec4(0.85f, 0.7f, 0.3f, 1.0f), "%s", status.c_str());
	}
	else
	{
		ImGui::TextColored(ImVec4(0.75f, 0.75f, 0.75f, 1.0f), "Spotify parado");
	}
	ImGui::TextColored(ImVec4(0.55f, 0.55f, 0.55f, 1.0f), "eletrica %s | telemetria %s",
		snap.electrics_on ? "ligada" : "desligada",
		snap.telemetry_connected ? "ok" : "sem conexao");
	ImGui::TextColored(ImVec4(0.4f, 0.4f, 0.4f, 1.0f), "F9: %s painel", g_panel_expanded ? "recolher" : "expandir");
	ImGui::EndGroup();
}

// Barra de progresso da faixa atual (posicao/duracao - ver
// ets2::Snapshot::position_ms/duration_ms, preenchidos pelos dois backends
// - GetTimelineProperties no Smtc, eventos do Player no Connect). So
// desenhada quando a duracao e conhecida; largura sempre igual a do resto
// do painel (`-FLT_MIN`), entao acompanha o auto-resize da janela.
void draw_progress(const ets2::Snapshot &snap)
{
	if (!snap.has_track || snap.duration_ms == 0)
		return;

	const float fraction = std::clamp(
		static_cast<float>(snap.position_ms) / static_cast<float>(snap.duration_ms), 0.0f, 1.0f);

	char overlay[32];
	std::snprintf(overlay, sizeof(overlay), "%u:%02u / %u:%02u",
		snap.position_ms / 60000u, (snap.position_ms / 1000u) % 60u,
		snap.duration_ms / 60000u, (snap.duration_ms / 1000u) % 60u);

	ImGui::Spacing();
	ImGui::ProgressBar(fraction, ImVec2(-FLT_MIN, 0), overlay);
}

// Slider de volume (0-100) - so aparece com o painel expandido, igual aos
// outros controles. `volume_ui` fica sincronizado com `snap.volume` (lido
// do backend ativo) sempre que o usuario nao estiver arrastando o slider
// agora (`IsItemActive`), pra nao brigar com o gesto do usuario caso o
// volume mude por fora (outro app controlando o mesmo dispositivo Connect,
// por exemplo) no meio do arrasto.
void draw_volume(const ets2::Snapshot &snap)
{
	static int volume_ui = -1;
	if (volume_ui < 0)
		volume_ui = static_cast<int>(snap.volume);

	ImGui::SetNextItemWidth(-FLT_MIN);
	const bool changed = ImGui::SliderInt("##volume", &volume_ui, 0, 100, "Volume %d%%");
	if (changed)
		ets2_set_volume(static_cast<uint32_t>(volume_ui));
	else if (!ImGui::IsItemActive())
		volume_ui = static_cast<int>(snap.volume);
}

// Botao so com um icone (glifo ASCII/Latin-1 - o unico intervalo de
// caractere garantido na fonte padrao do ImGui sem embutir uma fonte de
// icones a parte) + tooltip com o nome por extenso ao passar o mouse.
bool icon_button(const char *icon, const char *tooltip)
{
	const bool clicked = ImGui::Button(icon);
	if (ImGui::IsItemHovered())
		ImGui::SetTooltip("%s", tooltip);
	return clicked;
}

void draw_controls()
{
	if (icon_button("|<<", "Anterior"))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Previous), nullptr, 0);
	ImGui::SameLine();
	if (icon_button(">||", "Play / Pause"))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::PlayPause), nullptr, 0);
	ImGui::SameLine();
	if (icon_button(">>|", "Proxima"))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Next), nullptr, 0);
}

void draw_playlists()
{
	ets2::Playlist playlists[kMaxPlaylistsShown];
	size_t playlist_count = 0;
	if (!ets2_list_playlists(playlists, kMaxPlaylistsShown, &playlist_count) || playlist_count == 0)
		return;

	ImGui::Spacing();
	ImGui::TextColored(ImVec4(0.6f, 0.6f, 0.6f, 1.0f), "Playlists");
	const size_t shown = playlist_count < kMaxPlaylistsShown ? playlist_count : kMaxPlaylistsShown;
	for (size_t i = 0; i < shown; ++i)
	{
		const std::string name(reinterpret_cast<const char *>(playlists[i].name), playlists[i].name_len);
		const std::string uri(reinterpret_cast<const char *>(playlists[i].uri), playlists[i].uri_len);
		ImGui::PushID(static_cast<int>(i));
		if (ImGui::Button(name.empty() ? "(sem nome)" : name.c_str()))
		{
			ets2_send_command(
				static_cast<uint32_t>(ets2::CommandKind::PlayUri),
				reinterpret_cast<const uint8_t *>(uri.data()), uri.size());
		}
		ImGui::PopID();
	}
}

// Checa as hotkeys globais (Ctrl+PageUp/PageDown/Insert/Delete) e manda o
// comando correspondente. Roda todo frame independente do painel estar
// aberto ou fechado - diferente do toggle do painel (kTogglePanelKey), que
// so faz sentido dentro do desenho do proprio painel.
void check_global_hotkeys(reshade::api::effect_runtime *runtime)
{
	if (!runtime->is_key_down(VK_CONTROL))
		return;
	if (runtime->is_key_pressed(kHotkeyNext))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Next), nullptr, 0);
	if (runtime->is_key_pressed(kHotkeyPrevious))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Previous), nullptr, 0);
	if (runtime->is_key_pressed(kHotkeyPlay))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Play), nullptr, 0);
	if (runtime->is_key_pressed(kHotkeyPause))
		ets2_send_command(static_cast<uint32_t>(ets2::CommandKind::Pause), nullptr, 0);
}

void draw_osd(reshade::api::effect_runtime *runtime)
{
	// Inicio preguicoso (lazy): so na primeira vez que um frame e desenhado,
	// bem depois do DllMain/DLL_PROCESS_ATTACH ja ter terminado (loader lock
	// solto). Criar a thread motor (Rust, SMTC+telemetria) direto do
	// DllMain arriscaria deadlock com outras DLLs carregando ao mesmo tempo.
	if (!g_engine_started)
		g_engine_started = ets2_engine_start();

	ets2::Snapshot snap{};
	if (!ets2_poll_snapshot(&snap))
		return; // motor ainda nao terminou de iniciar

	check_global_hotkeys(runtime);

	if (runtime->is_key_pressed(kTogglePanelKey))
		g_panel_expanded = !g_panel_expanded;

	if (snap.thumb_generation != 0)
		ensure_album_art_texture(runtime->get_device(), snap.thumb_width, snap.thumb_height, snap.thumb_generation);

	ImGuiWindowFlags flags = ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoFocusOnAppearing | ImGuiWindowFlags_NoNav;
	if (!g_panel_expanded)
		flags |= ImGuiWindowFlags_NoInputs;

	ImGui::SetNextWindowBgAlpha(0.75f);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(12, 10));
	ImGui::Begin("##ets2_spotify_panel", nullptr, flags);

	draw_now_playing(snap);
	draw_progress(snap);

	if (g_panel_expanded)
	{
		ImGui::Separator();
		draw_controls();
		ImGui::Spacing();
		draw_volume(snap);
		draw_playlists();

		// Enquanto o painel estiver aberto, bloqueia o input do proprio jogo
		// (clicar nos botoes nao deve mexer no volante/camera por baixo).
		// Chamado todo frame que o painel estiver expandido, como a doc do
		// ReShade pede.
		runtime->block_input_next_frame();
	}

	ImGui::End();
	ImGui::PopStyleVar();
}

} // namespace

extern "C" __declspec(dllexport) const char *NAME = "ETS2 x Spotify";
extern "C" __declspec(dllexport) const char *DESCRIPTION =
	"Controle o Spotify direto de dentro do caminhao: play/pause, anterior/proxima, volume, "
	"progresso da faixa, playlists salvas, capa do album e faixa/artista atuais, com hotkeys "
	"globais (Ctrl+PageUp/PageDown/Insert/Delete). Backend Connect: nao precisa de nenhum app "
	"separado, nem do Spotify Desktop aberto.";

BOOL APIENTRY DllMain(HMODULE hModule, DWORD fdwReason, LPVOID)
{
	switch (fdwReason)
	{
	case DLL_PROCESS_ATTACH:
		if (!reshade::register_addon(hModule))
			return FALSE;
		// "OSD" registra no HUD sempre-visivel do ReShade (relogio/FPS),
		// entao nosso painel aparece mesmo com o menu do ReShade fechado -
		// ver secao "Overlays" do REFERENCE.md do ReShade.
		reshade::register_overlay("OSD", &draw_osd);
		break;
	case DLL_PROCESS_DETACH:
		// So SINALIZA a thread motor pra parar - `ets2_engine_shutdown`
		// (ffi.rs) e deliberadamente nao-bloqueante, nao da join/espera em
		// nenhuma thread daqui. DllMain roda com o loader lock do processo
		// preso durante TODO o DLL_PROCESS_DETACH; esperar (ou pior, criar)
		// qualquer thread de fundo aqui dentro arrisca deadlock se ela
		// precisar do loader lock por baixo dos panos (rede via
		// reqwest/hyper/tokio no backend Connect faz isso o tempo todo:
		// resolucao de DNS, TLS/schannel, etc.) - foi exatamente isso que
		// travava o jogo ao fechar (so morria via kill forcado pela Steam).
		// A thread motor (e o backend Connect dentro dela) terminam
		// sozinhas, de forma inteiramente assincrona - ver o comentario em
		// `ets2_engine_shutdown`.
		if (g_engine_started)
			ets2_engine_shutdown();
		// NOTA: a textura da capa (g_art_resource/g_art_view) nao e
		// destruida aqui de proposito - nao ha garantia de que o
		// `device`/swapchain ainda estejam validos neste ponto do unload,
		// e mexer neles fora do hook de renderizacao e arriscado. E uma
		// unica textura pequena; o pior caso e ela ficar alocada ate o
		// jogo fechar de verdade (processo inteiro morrendo libera tudo).
		reshade::unregister_addon(hModule);
		break;
	}
	return TRUE;
}
