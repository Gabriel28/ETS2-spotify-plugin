@echo off
REM Compila overlay_addon.cpp com o MSVC. Precisa do "Desktop development
REM with C++" instalado via Visual Studio Installer (Community/Build Tools,
REM qualquer edicao serve).
setlocal enabledelayedexpansion
cd /d "%~dp0"

set VSWHERE="%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set VCVARS=

REM IMPORTANTE: usar !VSWHERE! (expansao adiada), nao %VSWHERE% - o valor
REM contem "(x86)" (de %ProgramFiles(x86)%), e expansao imediata de uma
REM variavel com parenteses dentro de um bloco if/for tambem parentesado
REM quebra o parser do cmd (o cmd conta parenteses no texto antes de
REM substituir %VAR%, entao os parenteses do VALOR sao contados como se
REM fossem do bloco) - com !VSWHERE! a substituicao so acontece na hora de
REM executar, depois do bloco inteiro ja ter sido interpretado.
if exist !VSWHERE! (
	for /f "usebackq tokens=*" %%i in (`!VSWHERE! -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do (
		set VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat
	)
)

if not defined VCVARS (
	for %%e in (Community Professional Enterprise BuildTools) do (
		if exist "C:\Program Files\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat" (
			set VCVARS=C:\Program Files\Microsoft Visual Studio\2022\%%e\VC\Auxiliary\Build\vcvars64.bat
		)
	)
)

if not defined VCVARS (
	echo Nao achei o Visual Studio ^(vcvars64.bat^). Instale o "Desktop development with C++" pelo Visual Studio Installer.
	exit /b 1
)

call "!VCVARS!" >nul
if errorlevel 1 (
	echo Falha ao inicializar o ambiente do MSVC ^(vcvars64.bat^).
	exit /b 1
)

REM Compila a staticlib Rust (src/ffi.rs e o resto do "motor" - SMTC,
REM telemetria, audio, config) e gera ..\target\release\ets2_spotify_core.lib,
REM que o link abaixo embute na mesma DLL do addon.
REM Parenteses literais aqui (e em qualquer outro echo dentro de um bloco
REM if/for) tem que ser escapados com ^( ^) - sem isso o cmd conta esses
REM parenteses como parte do bloco e quebra o parser (mesmo bug de
REM parenteses do !VSWHERE!, so que com texto literal em vez do valor de
REM uma variavel).
where cargo >nul 2>&1
if errorlevel 1 (
	echo Nao achei o cargo no PATH. Instale o Rust ^(https://rustup.rs^).
	exit /b 1
)

pushd ..
cargo build --release --lib
set CARGO_EXIT=%ERRORLEVEL%
popd
if %CARGO_EXIT% neq 0 (
	echo Build da staticlib Rust falhou.
	exit /b %CARGO_EXIT%
)

set RUST_LIB=..\target\release\ets2_spotify_core.lib
if not exist "%RUST_LIB%" (
	echo Nao achei %RUST_LIB% depois do cargo build - confira o nome do [lib] no Cargo.toml.
	exit /b 1
)

REM Libs do Windows que a crate `windows` (SMTC/COM/WinRT) e a staticlib
REM Rust em geral resolvem em tempo de link - descobertas empiricamente via
REM erros de "unresolved external symbol"; adicione mais aqui se aparecer
REM algum novo.
cl.exe /nologo /LD /EHsc /std:c++17 /W3 /I include overlay_addon.cpp ^
	/link /OUT:overlay_addon.dll "%RUST_LIB%" ^
	runtimeobject.lib ole32.lib oleaut32.lib user32.lib ntdll.lib ^
	advapi32.lib bcrypt.lib userenv.lib ws2_32.lib ncrypt.lib crypt32.lib ^
	secur32.lib propsys.lib mmdevapi.lib
set EXIT_CODE=%ERRORLEVEL%
del /q overlay_addon.obj overlay_addon.exp overlay_addon.lib >nul 2>&1
if %EXIT_CODE% neq 0 (
	echo Build falhou.
	exit /b %EXIT_CODE%
)
echo.
echo Build ok: overlay_addon.dll
echo Renomeie para overlay_addon.addon e copie pra pasta de addons do ReShade dentro do jogo.
