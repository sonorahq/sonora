# common
common-on = Ligado
common-off = Desligado
common-left = Esquerda
common-right = Direita
common-search = Pesquisar
common-unknown = Desconhecido
common-not-provided = Não indicado
common-not-available = Indisponível
common-cancel = Cancelar
common-save = Guardar
common-delete = Eliminar
common-play = Tocar
common-more = Mais
common-previous = Anterior
common-next = Seguinte
common-dismiss = Dispensar
common-clear = Limpar
number-group = { "." }

# navigation
nav-history = Histórico
nav-home = Início
nav-search = Pesquisar
nav-library = A sua biblioteca
nav-settings = Definições
nav-songs = Músicas
nav-albums = Álbuns
nav-playlists = Playlists
nav-artists = Artistas
nav-local = Música Local
nav-back = Voltar
nav-forward = Avançar
nav-sidebar = Alternar barra lateral
nav-sidebar-right = Mostrar ou ocultar letras e fila
nav-pinned = Fixados
nav-unpin = Desafixar
nav-pin-hint = Solte aqui para fixar
library-liked-songs = Favoritas
library-no-songs = Nenhuma favorita ainda
library-no-albums = Nenhum álbum guardado ainda
library-no-playlists = Nenhuma playlist ainda
library-no-artists = Nenhum artista favorito ainda
library-no-local-songs = Nenhuma música importada encontrada
library-no-local-albums = Nenhum álbum importado encontrado
library-no-local-artists = Nenhum artista importado encontrado
library-no-local-playlists = Nenhuma playlist local ainda
library-no-catalog-songs = Nenhuma música encontrada
library-no-catalog-albums = Nenhum álbum encontrado
library-no-catalog-artists = Nenhum artista encontrado
library-no-matches = Nenhum resultado
library-not-loaded = A sua biblioteca não carregou
library-scanning = A procurar nas suas pastas…
library-scanning-found = A procurar nas suas pastas… { $found } até agora
library-scanning-progress = A ler as suas músicas… { $read } de { $found }
library-part-not-loaded = Esta parte da sua biblioteca não carregou
library-local-unconfigured = Configure a sua biblioteca local

# app menu
app-refresh = Atualizar
app-sign-out = Terminar sessão
app-quit = Sair da aplicação
app-settings = Definições…
app-hide = Ocultar Sonora
app-hide-others = Ocultar outros
app-show-all = Mostrar tudo
app-edit = Editar
app-cut = Cortar
app-copy = Copiar
app-paste = Colar
app-select-all = Selecionar tudo
app-window = Janela
app-close-window = Fechar janela
app-minimize = Minimizar
app-zoom = Zoom

# tray menu
tray-show = Mostrar Sonora
tray-play = Tocar
tray-pause = Pausar

# table columns
column-played-at = Tocada em
column-index = #
column-title = Título
column-artist = Artista
column-album = Álbum
column-genre = Género
column-date-added = Data de adição
column-added-by = Adicionado por
column-modified = Modificado
column-length = Duração
column-plays = Reproduções
column-name = Nome
column-owner = Proprietário
column-year = Ano
column-tracks = Faixas

# track menu
menu-new-playlist = Nova playlist
menu-import-playlist = Importar playlist…
menu-edit-tags = Editar tags
menu-no-playlists = Nenhuma playlist
menu-search-playlists = Pesquisar playlist
menu-add-to-library = Adicionar às Favoritas
menu-remove-from-library = Remover das Favoritas
menu-remove-from-playlist = Remover da playlist
menu-remove-tracks-from-playlist = { $count ->
    [one] Remover { $count } faixa da playlist
   *[other] Remover { $count } faixas da playlist
}
menu-remove-from-history = Remover do histórico
menu-remove-tracks-from-history = { $count ->
    [one] Remover { $count } faixa do histórico
   *[other] Remover { $count } faixas do histórico
}
menu-delete-track-file = Eliminar ficheiro da faixa
menu-delete-track-files = Eliminar { $count } ficheiros de faixa
menu-download = Transferir
menu-remove-download = Remover transferência
menu-downloading = A transferir. Clique para parar
menu-play-next = Tocar a seguir
menu-play-tracks-next = { $count ->
    [one] Tocar { $count } faixa a seguir
   *[other] Tocar { $count } faixas a seguir
}
menu-add-to-queue = Adicionar à fila
menu-add-tracks-to-queue = { $count ->
    [one] Adicionar { $count } faixa à fila
   *[other] Adicionar { $count } faixas à fila
}
menu-play-last = Tocar por último
menu-play-tracks-last = { $count ->
    [one] Tocar { $count } faixa por último
   *[other] Tocar { $count } faixas por último
}
menu-song-radio = Ir para a rádio da música
menu-go-to-album = Ir para o álbum
menu-go-to-artist = Ir para o artista
menu-view-details = Ver detalhes
menu-copy-link = Copiar link
menu-cut = Cortar
menu-copy = Copiar
menu-paste = Colar
menu-select-all = Selecionar tudo
menu-remove-from-queue = Remover da fila
menu-edit-playlist = Editar playlist
menu-delete-playlist = Eliminar playlist
menu-export-playlist = Exportar playlist…
menu-add-playlist-to-library = Adicionar à Biblioteca
menu-remove-playlist-from-library = Remover da Biblioteca
menu-make-playlist-public = Tornar pública
menu-make-playlist-private = Tornar privada
menu-play-artist = Tocar artista

# playlist editor
playlist-name-placeholder = Nome da playlist
playlist-create-title = Criar playlist
playlist-edit-title = Editar playlist
playlist-cover-choose = Escolher imagem
playlist-cover-remove = Remover capa
playlist-cover-unopenable = O Sonora não consegue abrir este ficheiro
playlist-cover-too-large = Esta imagem tem mais de 40 MB
playlist-cover-not-a-picture = Este ficheiro não é uma imagem JPEG, PNG ou WebP
playlist-delete-title = Eliminar playlist
playlist-delete-confirm = Eliminar "{ $name }"? Isto não pode ser anulado.
playlist-again-title = Adicionar novamente?
playlist-again-confirm = Esta faixa já está em "{ $name }". Adicionar outra cópia?
playlist-again-add = Adicionar novamente

# confirm
confirm-remove-library-title = Remover da biblioteca
confirm-remove-playlist-title = Remover da playlist
confirm-remove-history-title = Remover do histórico
confirm-remove-songs = { $count ->
    [one] Remover esta música da sua biblioteca?
   *[other] Remover { $count } músicas da sua biblioteca?
}
confirm-remove-playlist-songs = { $count ->
    [one] Remover esta música da playlist?
   *[other] Remover { $count } músicas da playlist?
}
confirm-remove-history-songs = { $count ->
    [one] Remover esta música do histórico de reprodução?
   *[other] Remover { $count } músicas do histórico de reprodução?
}
confirm-remove-albums = { $count ->
    [one] Remover este álbum da sua biblioteca?
   *[other] Remover { $count } álbuns da sua biblioteca?
}
confirm-remove-artists = { $count ->
    [one] Remover este artista das Favoritas?
   *[other] Remover { $count } artistas das Favoritas?
}
confirm-remove-playlists = { $count ->
    [one] Remover esta playlist da sua biblioteca?
   *[other] Remover { $count } playlists da sua biblioteca?
}
confirm-delete-track-files-title = Eliminar ficheiros das faixas?
confirm-delete-track-files = Os ficheiros serão eliminados permanentemente do disco. Isto não pode ser anulado.

# queue panel
queue-title = Fila
queue-history = Histórico
queue-now-playing = Em reprodução
queue-from = De
queue-from-radio = Rádio de { $name }
queue-next-in-queue = A seguir na fila
queue-up-next = A seguir
queue-reset = Repor
queue-clear = Limpar
queue-added-by = Adicionado por { $name }
queue-empty = A sua fila está vazia
queue-similar = Faixas semelhantes
queue-radio = Reprodução automática de faixas semelhantes
queue-return-playing = Voltar ao que está a tocar

# player bar
player-nothing-playing = Nada em reprodução
player-percent = { $value }%
player-shuffle = Aleatório
player-repeat = Repetir
player-repeat-all = Repetir tudo
player-repeat-one = Repetir uma
player-mute = Silenciar
player-unmute = Ativar som
player-previous = Faixa anterior
player-next = Faixa seguinte
player-fullscreen = Ecrã inteiro
player-fullscreen-leave = Sair do ecrã inteiro
devices-this-computer = Este computador
devices-jam-join = Entrar numa Jam…
devices-jam-join-title = Entrar numa Jam
devices-jam-join-detail = Cole o link que um amigo partilhou consigo
devices-jam-paste = Colar
devices-jam-link-hint = Link da Jam
devices-jam-start = Iniciar uma Jam
devices-jam-copy = Copiar link da Jam
devices-jam-code-menu = Mostrar código QR
devices-jam-code-title = Digitalize para entrar na Jam de { $host }
devices-jam-code-title-anonymous = Digitalize para entrar na Jam
devices-jam-code-close = Fechar
devices-jam-leave = Sair da Jam
devices-jam-end = Terminar Jam
devices-jam-more = +{ $count }
devices-jam-host = Anfitrião
devices-jam-you = Você
devices-jam-remove = Remover
devices-jam-invite-title = Entrar na Jam de { $host }?
devices-jam-invite-title-anonymous = Entrar nesta Jam?
devices-jam-invite-detail = { $count ->
    [one] { $count } pessoa já está nesta Jam.
   *[other] { $count } pessoas já estão nesta Jam.
}
devices-jam-invite-join = Entrar
fullscreen-artwork = Capa

# filters
filter-history = Filtrar histórico de reprodução
history-empty = As faixas que tocar aparecerão aqui.
history-not-loaded = O histórico de reprodução não pôde ser carregado.
history-clear = Limpar histórico
history-clear-title = Limpar histórico de reprodução
history-clear-confirm = Todas as reproduções serão removidas deste dispositivo. Isto não pode ser anulado.
filter-library = Filtrar a sua biblioteca
filter-album = Filtrar faixas do álbum
filter-reset = Repor filtros
filter-duration = Duração
filter-year = Ano
filter-hide-explicit = Ocultar explícitas
filter-playable = Apenas reproduzíveis
filter-favorites = Apenas favoritas
filter-downloaded = Transferidas
filter-owned = Suas

# view
view-list = Lista
view-cards = Grelha

# toolbar
tool-sort = Ordenar
tool-filters = Filtros

# login
login-signed-out = Inicie sessão para carregar a sua biblioteca musical
login-restoring = A verificar a sua sessão guardada…
login-authorizing = A aguardar autorização no seu navegador…
login-signed-in = Sessão iniciada como { $name }
login-failed-title = Falha ao iniciar sessão
login-problem-region = O Spotify não abrirá uma sessão a partir do país onde está. Inicie sessão a partir do seu país de origem ou altere o país na sua conta do Spotify.
login-problem-credentials = A sua sessão guardada do Spotify já não é válida. Inicie sessão novamente para continuar.
login-problem-network = O Sonora não conseguiu aceder ao serviço de música. Verifique a sua ligação à internet e tente novamente.
login-problem-cancelled = Fechou a página do navegador antes de aprovar o início de sessão. Recomece para concluir.
login-problem-refused = O Spotify recusou o início de sessão. Aguarde um momento e tente novamente.
login-problem-premium = O Sonora usa o Spotify Premium para reproduzir, e esta conta não tem esse plano. Inicie sessão com uma conta Premium para continuar.
login-sign-in = Iniciar sessão com { $provider }
login-connect-cookies = Colar cookies manualmente
login-cookie-open = Abrir { $provider }
login-cookie-submit = Continuar
login-cookie-hint = Cole o cabeçalho Cookie aqui
login-cookie-step-1 = Aceda a music.youtube.com e certifique-se de que tem sessão iniciada. Funciona melhor numa janela anónima.
login-cookie-step-2 = Prima F12, abra o separador Rede e recarregue a página.
login-cookie-step-3 = Selecione qualquer pedido chamado "browse" ou "next".
login-cookie-step-4 = Em Cabeçalhos, encontre Cookie em Cabeçalhos do pedido, clique com o botão direito e copie o valor.
login-cookie-step-note = Cole o valor completo abaixo: o painel Cookies do pedido não é suficiente, pois o valor tem de conter SAPISID e __Secure-3PAPISID.
login-cookie-header-title = Cole os seus cookies do { $provider } para concluir o início de sessão
login-cookie-apple-step-3 = Selecione qualquer pedido para { $site }.
login-cookie-apple-note = Cole o valor completo, incluindo { $cookie }.
login-cookie-named-title = Cole o seu cookie do { $provider } para concluir o início de sessão
login-cookie-named-hint = Cole o valor do cookie aqui
login-cookie-named-step-1 = Aceda a { $site } e certifique-se de que tem sessão iniciada.
login-cookie-named-step-2 = Prima F12 e abra o separador Armazenamento no Firefox ou o separador Aplicação no Chrome.
login-cookie-named-step-3 = Expanda Cookies, selecione { $site } e encontre o cookie chamado { $cookie }.
login-cookie-named-step-4 = Faça duplo clique no valor e copie-o.
login-cookie-named-note = Só o valor de { $cookie } já basta. Um cabeçalho Cookie inteiro copiado do separador Rede também funciona.
login-window-title = Iniciar sessão no { $provider }
login-use = Usar { $provider }
login-guest-title = Modo convidado
login-guest-use = Usar modo convidado
login-guest-detail = Navegue e toque sem uma conta. A sua biblioteca, gostos e playlists ficam inacessíveis.
login-usage-consent = Ajude-nos a estimar quantas pessoas usam o Sonora.
login-device-code = Introduza este código em { $url }
login-server-title = Ligue-se ao seu servidor Subsonic
login-server-detail = Introduza o endereço de qualquer servidor Subsonic ou OpenSubsonic (Navidrome, Airsonic, Gonic, …) e inicie sessão com o seu utilizador e palavra-passe do servidor. A sessão permanece neste dispositivo.
login-server-hint = https://music.example.com
login-username-hint = Utilizador
login-password-hint = Palavra-passe
login-server-submit = Ligar
login-account-title = Escolha uma conta
login-account-detail = Esta sessão está iniciada em mais de uma conta Google. Escolha qual o Sonora deve usar.
login-choose-title = Iniciar sessão no { $provider }
login-choose-detail = Escolha como iniciar sessão no { $provider }.

# album and playlist pages
detail-playlist = Playlist

# play button
play-pause = Pausar
play-resume = Retomar
play-shuffle = Aleatório

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } ouvinte mensal
   *[other] { $value } ouvintes mensais
}
artist-popular = Populares
artist-popular-eyebrow = Explorar este artista
artist-popular-empty = Nada para tocar deste artista ainda
artist-popular-more = Mostrar tudo
artist-popular-less = Mostrar menos
artist-releases = Lançamentos
artist-releases-more = Mostrar tudo
artist-releases-less = Mostrar menos
artist-filter-all = Tudo
artist-filter-albums = Álbuns
artist-filter-singles = Singles
artist-filter-eps = EPs
artist-appears-on = Aparece em
album-also-like = Também pode gostar
album-tab-albums = Álbuns
album-tab-artists = Artistas
album-label = ℗ { $label }

# user profile page
user-eyebrow = Perfil
user-followers = { $count ->
    [one] { $value } seguidor
   *[other] { $value } seguidores
}
user-following = { $count ->
   *[other] { $value } a seguir
}
user-playlists = Playlists públicas
user-playlists-empty = Nenhuma playlist pública ainda

# release kinds
release-album = Álbum
release-single = Single
release-compilation = Compilação
release-ep = EP
release-audiobook = Audiolivro
release-podcast = Podcast
release-meta = { $year } • { $kind }

# home page
home-quick-picks = Escolhas rápidas
home-quick-picks-empty = Goste de algumas músicas e elas aparecerão aqui
home-recently-added = Adicionados recentemente
home-recently-played = Tocados recentemente
home-playlists = Playlists
home-favorite-albums = Álbuns favoritos
home-artists = Artistas
home-collection-albums = Álbuns da sua coleção

# search page
search-placeholder = O que quer ouvir?
search-browse = Explorar tudo
search-submit = Pesquisar
search-recent = Pesquisas recentes
search-forget = Remover do histórico
genre-empty = Nada para mostrar aqui ainda
search-best-match = Melhor resultado
search-no-matches = Nenhum resultado
search-results = Resultados
search-songs = Músicas
search-artists = Artistas
search-albums-playlists = Álbuns e playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } música na Biblioteca
       *[other] { $count } músicas na Biblioteca
    }
kind-song = Música
kind-artist = Artista
kind-album = Álbum
kind-playlist = Playlist

# song page
song-eyebrow = Música
song-view-album = Ver álbum
song-loading = A carregar informações da música…
song-about = Sobre esta música
song-album = Álbum
song-released = Lançada
song-streams = Reproduções
song-position = Posição
song-label = Editora
song-popularity = Popularidade
song-popularity-value = { $value }%
song-disc-track = Disco { $disc }, faixa { $track }
song-track = Faixa { $track }
song-credits = Créditos
song-performed-by = Interpretada por
song-details = Géneros e detalhes
song-genres = Géneros
song-language = Idioma
song-content = Conteúdo
song-explicit = Explícito
song-clean = Limpo
artist-about = Sobre o artista
artist-about-fallback = Explore as músicas e lançamentos populares do artista.
artist-about-open = Ir para o artista
song-copyright = © { $notice }

# song languages
language-ar = Árabe
language-de = Alemão
language-en = Inglês
language-es = Espanhol
language-fr = Francês
language-hi = Hindi
language-it = Italiano
language-ja = Japonês
language-ko = Coreano
language-pt = Português
language-ru = Russo
language-tr = Turco
language-uk = Ucraniano
language-zh = Chinês
language-zxx = Sem conteúdo linguístico

# counts
count-songs =
    { $count ->
        [one] { $count } música
       *[other] { $count } músicas
    }
count-tracks =
    { $count ->
        [one] { $count } faixa
       *[other] { $count } faixas
    }

# running times
runtime-seconds = { $seconds }s
runtime-minutes = { $minutes }min { $seconds }s
runtime-hours = { $hours }h { $minutes }min

# dates
date-just-now = Agora mesmo
date-minute-ago = Há um minuto
date-minutes-ago = Há { $count } minutos
date-today = Hoje às { $time }
date-yesterday = Ontem às { $time }
date-time = { $date } às { $time }
date-full = { $day } de { $month } de { $year }
month-1 = Jan
month-2 = Fev
month-3 = Mar
month-4 = Abr
month-5 = Mai
month-6 = Jun
month-7 = Jul
month-8 = Ago
month-9 = Set
month-10 = Out
month-11 = Nov
month-12 = Dez

# settings
settings-search = Pesquisar nas definições
settings-search-results = Resultados da pesquisa
# where a search result lives: a drill-in page under the page it opens from
settings-result-place = { $parent } › { $page }
settings-page-providers = Serviços
settings-page-appearance = Aparência
settings-page-player-bar = Barra do leitor
settings-player-bar-layout = Layout
settings-player-bar-layout-detail = Organizar as informações da faixa, os controlos de reprodução e a barra de progresso
settings-player-bar-classic = Clássico
settings-player-bar-compact = Compacto
settings-player-bar-centered = Centralizado
settings-player-bar-surface = Superfície
settings-player-bar-transparent = Barra do leitor transparente
settings-player-bar-transparent-detail = Deixar páginas e barras laterais deslizarem por baixo de uma barra do leitor fosca
settings-player-bar-transparent-needs-blur = Ative o Desfoque da interface para as páginas deslizarem por baixo de uma barra do leitor fosca
settings-page-now-playing = Em reprodução
settings-page-lyrics = Letras
settings-page-playback = Reprodução
settings-page-sound = Som
settings-page-storage = Armazenamento
settings-page-discord = Discord
settings-page-scrobbling = Scrobbling
settings-page-general = Geral
settings-page-about = Sobre
settings-tab-integrations = Integrações
settings-about-name = Sonora
settings-page-lyrics-sources = Fontes de letras
settings-page-music-folders = Pastas de música
settings-group-services = Serviços de streaming
settings-group-local-files = Ficheiros locais
settings-group-background = Fundo
settings-group-controls = Controlos
settings-group-display = Apresentação
settings-group-sources = Fontes
settings-group-lyrics-order = Prioridade
settings-group-playing = Reprodução
settings-group-cache = Faixas tocadas
settings-group-downloads = Transferências
settings-group-protected = Reprodução protegida
settings-group-output = Saída
settings-group-status = Estado
settings-group-startup = Arranque
settings-group-context-menus = Menus de contexto
key-ctrl = Ctrl
key-super = Super
key-alt = Alt
key-shift = Shift
key-escape = Esc
key-enter = Enter
key-space = Espaço
key-tab = Tab
key-backspace = Backspace
key-delete = Delete
settings-lyrics-sources = Fontes de letras
settings-local-folder-count = { $count ->
    [one] { $count } pasta
   *[other] { $count } pastas
    }
settings-local-folder-none = Nenhuma pasta ainda
settings-license-short = GPL-3.0 ou posterior
settings-theme = Tema
settings-theme-detail = Escolha a paleta de cores da aplicação
settings-opacity = Opacidade
settings-opacity-detail = Ajuste a opacidade do fundo da aplicação
settings-opacity-value = { $percent }%
settings-theme-folder = Abrir configuração
settings-theme-unavailable = Este tema só pode ser aplicado com o tema adaptável desativado
settings-adaptive = Tema adaptável
settings-adaptive-detail = Tingir a paleta com a capa do álbum em reprodução
settings-ambient = Fundo ambiente
settings-ambient-detail = Preencher a vista Em reprodução com cores tiradas da capa
settings-ambient-motion = Movimento ambiente
settings-ambient-motion-detail = Fazer as cores ambiente se moverem em vez de ficarem paradas
settings-visualizer = Visualizador
settings-visualizer-detail = Mostrar barras de espectro atrás da capa em ecrã inteiro
settings-visualizer-style-none = Desligado
settings-visualizer-style-bars = Barras
settings-visualizer-style-wave = Onda
settings-visualizer-style-both = Barras e onda
settings-visualizer-absolute = Ignorar volume
settings-visualizer-absolute-detail = Desenhar o espectro no nível da própria faixa, seja qual for o volume do Sonora
settings-preview-title = Título da música
settings-preview-artist = Artista
settings-preview-album = Álbum
settings-fullscreen-controls-autohide = Ocultar controlos em ecrã inteiro
settings-fullscreen-controls-autohide-detail = Desvanecer os controlos de reprodução quando o ecrã inteiro está inativo
settings-os-fullscreen = Usar ecrã inteiro do sistema
settings-os-fullscreen-detail = Fazer a vista Em reprodução ocupar o ecrã inteiro, como faz o F11
settings-icons = Pacote de ícones
settings-icons-detail = Escolha o conjunto de ícones usado na interface
settings-motion = Reduzir movimento
settings-motion-detail = Ignorar animações e transições da interface
settings-pace = Velocidade da animação
settings-pace-detail = A velocidade das animações da interface
settings-saver = Limitar FPS da janela inativa
settings-saver-detail = As alterações são aplicadas após reiniciar o Sonora.
settings-corners = Cantos
settings-corners-detail = Quão arredondados são as superfícies e os controlos
settings-blur = Desfoque da interface
settings-blur-detail = Aplicar vidro fosco a menus e controlos flutuantes
settings-blur-window = Desfocar a janela
settings-blur-window-detail = Desenhar a janela sobre um ambiente de trabalho desfocado. Precisa de opacidade abaixo de 100%
settings-font = Tamanho do tipo de letra
settings-font-detail = Tamanho base do texto, tudo o resto acompanha
settings-font-value = { $size } px
settings-startup = Mostrar no arranque
settings-startup-detail = O ecrã que o Sonora abre ao iniciar
settings-startup-no-guest = Indisponível no modo convidado
settings-entries = Itens da barra lateral
settings-entries-detail = As seções listadas na barra lateral
settings-entries-pick = Escolher itens
settings-language = Idioma
settings-language-detail = O idioma usado pela interface do Sonora
settings-language-system = Sistema
settings-language-search = Pesquisar idioma
settings-language-none = Nenhum idioma encontrado
settings-typeface = Tipo de letra
settings-typeface-detail = O tipo de letra usado pelo Sonora na interface
settings-typeface-system = Predefinido
settings-typeface-search = Pesquisar tipo de letra
settings-typeface-none = Nenhum tipo de letra encontrado
settings-server-side-decorations = Decorações do servidor
settings-server-side-decorations-detail = Deixar o compositor desenhar a barra de título, a borda e a sombra
settings-typeface-loading = A carregar…
settings-window-controls = Botões da janela
settings-window-controls-detail = Desenhar minimizar, maximizar e fechar na barra de título
settings-traffic-light-controls = Botões estilo semáforo
settings-traffic-light-controls-detail = Desenhar minimizar, maximizar e fechar como pontos coloridos
settings-window-rounding = Cantos da janela
settings-window-rounding-detail = Quão arredondados são os cantos da janela
settings-controls-side = Lado dos botões
settings-controls-side-detail = Em que extremidade da barra de título ficam os botões
settings-close-to-tray = Continuar a tocar ao fechar
settings-close-to-tray-detail = Manter o Sonora na área de notificação e continuar a tocar depois de a janela fechar
settings-tray-icon = Mostrar na área de notificação
settings-tray-icon-detail = Colocar um ícone com controlos de reprodução na área de notificação
settings-spotify-connect = Mostrar como dispositivo Spotify
settings-spotify-connect-name = Nome do dispositivo
settings-spotify-connect-name-detail = Como o Sonora aparece na lista de dispositivos das aplicações do Spotify
settings-spotify-connect-name-sonora = Sonora
settings-spotify-connect-name-both = Sonora (nome do computador)
settings-spotify-connect-name-computer = Nome do computador
settings-spotify-connect-name-custom = Personalizado
settings-spotify-connect-custom = Nome personalizado
settings-spotify-connect-custom-detail = O nome a apresentar. Se ficar vazio, são usados Sonora e o nome do computador
settings-spotify-connect-custom-hint = Nome do dispositivo
settings-spotify-connect-detail = Listar o Sonora no seletor de dispositivos das aplicações do Spotify, para o controlar pelo telemóvel e o Spotify mostrar o que toca
settings-discord = Mostrar no Discord
settings-discord-detail = Mostrar a faixa que está a tocar no seu perfil do Discord
settings-discord-name = Nome do estado
settings-discord-name-detail = Como o estado se chama, depois do "A ouvir" que os seus amigos veem
settings-discord-name-sonora = Sonora
settings-discord-name-provider = Serviço
settings-discord-name-music = Música
settings-discord-name-title = Título
settings-discord-name-artist = Artista
settings-discord-name-artist-title = Artista - Título
settings-discord-show-paused = Mostrar quando em pausa
settings-discord-show-paused-detail = Manter o estado no seu perfil do Discord enquanto a faixa está em pausa
settings-discord-badge = Mostrar o selo do serviço
settings-discord-badge-detail = Marcar o estado com um pequeno ícone do serviço de onde veio a faixa
settings-discord-anonymous = Ocultar detalhes
settings-discord-anonymous-detail = Dizer apenas que há música a tocar, sem o título, o artista ou a capa
settings-discord-hide-on-connect = Ocultar durante o Spotify Connect
settings-discord-hide-on-connect-detail = Deixar o estado a cargo do próprio Spotify enquanto o Sonora toca como dispositivo Spotify ou outro dispositivo está a tocar
settings-discord-buttons = Botões
settings-discord-buttons-detail = Links abaixo do estado para os seus amigos abrirem, um para a faixa no serviço dela e outro para o Sonora
settings-discord-buttons-pick = Escolher botões
settings-discord-preview-listening = A ouvir { $name }
settings-discord-preview-music = Música
settings-discord-preview-elapsed = { $time } decorridos
settings-discord-preview-listen-on = Ouvir no { $provider }
# the Discord status when the track is left out of it
discord-listening = A ouvir música
get-sonora = Transferir o Sonora
settings-fullscreen-controls-autohide-automatic = Automático
settings-fullscreen-controls-autohide-always-hidden = Sempre ocultos
settings-fullscreen-controls-autohide-always-shown = Sempre visíveis
settings-normalisation = Normalizar volume
settings-normalisation-detail = Mantém as faixas num volume consistente
settings-gapless = Reprodução contínua
settings-gapless-detail = Toca uma faixa após a outra sem pausa, como um álbum foi sequenciado
settings-keep-tracks = Manter faixas tocadas
settings-keep-tracks-detail = Guarda o que toca, associado a este computador, para que tocar de novo não transfira nada. Desativar isto apaga o que foi guardado e não mexe nas suas transferências. O Spotify segue a mudança na próxima vez que o Sonora iniciar
settings-cache-size = Tamanho do cache
settings-cache-size-detail = Quanto espaço as faixas que tocou podem ocupar. As tocadas há mais tempo libertam espaço primeiro, e as transferências não entram na conta
settings-cache-size-value = { $count } GB
settings-cache-used = Espaço usado
settings-cache-used-detail = Quanto as faixas mantidas após tocar ocupam neste computador. Limpá-las não mexe nas suas transferências
settings-cache-clear = Limpar
settings-downloads-used = Espaço usado
settings-downloads-used-detail = Quanto as músicas que transferiu ocupam neste computador
settings-offline-mode = Modo offline
settings-offline-mode-detail = Mostrar e tocar apenas o que transferiu
settings-downloads-remove = Remover tudo
settings-size-gb = { $size } GB
settings-size-mb = { $size } MB
settings-size-measuring = A medir…
settings-stream-quality = Qualidade de streaming
settings-stream-quality-detail = Níveis mais altos soam melhor e usam mais dados. Quando o seu plano ou uma faixa não chega a esse nível, toca o nível abaixo
settings-stream-quality-hi-res = Hi-Res
settings-stream-quality-lossless = Sem perdas
settings-stream-quality-high = Alta
settings-stream-quality-normal = Normal
settings-stream-quality-low = Baixa
settings-stream-quality-flac = FLAC, { $depth } bits, { $rate } kHz
settings-stream-quality-flac-up-to = FLAC, até { $depth } bits, { $rate } kHz
settings-stream-quality-lossy = { $codec }, { $kbps } kbps
settings-stream-quality-lossy-up-to = { $codec }, até { $kbps } kbps
settings-stream-format = Formato do stream
settings-stream-format-detail = Para qual formato o servidor converte cada faixa antes de enviá-la
settings-stream-format-original = Original
settings-stream-format-mp3 = MP3
settings-stream-format-opus = Opus
settings-stream-format-aac = AAC
settings-stream-bitrate = Taxa de bits máxima
settings-stream-bitrate-detail = O servidor converte qualquer taxa acima disso para esta
settings-stream-bitrate-unlimited = Sem limite
settings-stream-bitrate-value = { $kbps } kbps
settings-stream-bitrate-hourly = Cerca de { $megabytes } MB por hora
settings-stream-bitrate-unlimited-detail = Cada ficheiro na sua própria taxa de bits
settings-output-device = Dispositivo de saída
settings-output-device-detail = Dispositivo de áudio onde o Sonora toca
settings-output-device-system = Predefinição do sistema
settings-output-device-missing = Dispositivo guardado (não ligado)
settings-output-device-direct = Direto para o hardware
settings-sample-rate = Taxa de amostragem
settings-sample-rate-detail = A taxa em que o dispositivo opera. O Sonora converte cada faixa para ela
settings-sample-rate-value = { $khz } kHz
settings-sample-rate-change = Alterar
settings-buffer = Tamanho do buffer
settings-buffer-detail = Quanto áudio o Sonora entrega ao dispositivo de cada vez
settings-output-idle = Sem reprodução
settings-output-device-loading = A carregar dispositivos…
settings-output-failed = Dispositivo não suportado
settings-output-failed-detail = Erro: { $error }
settings-signal-source = Fonte
settings-signal-normalized = Normalização
settings-signal-gain = { $db } dB
settings-signal-resampled = Reamostragem
settings-signal-khz = { $khz } kHz
settings-signal-kbps = { $kbps } kbps
settings-signal-bits = { $depth } bits
settings-signal-source-detail = { $rate } · { $quality }
settings-sleep = Temporizador de suspensão
settings-sleep-detail = Deixa a música parar sozinha depois de um tempo, para o embalar até adormecer
settings-stay-awake = Manter ativo durante a reprodução
settings-stay-awake-detail = Impedir a suspensão durante a reprodução e manter o ecrã ligado com a vista Em reprodução aberta
settings-sleep-configure = Configurar…
settings-sleep-off = Desligado
settings-sleep-end-of-track = Fim da faixa
settings-sleep-minutes = { $count } min
settings-widevine = Módulo Widevine
settings-widevine-detail = As faixas do Apple Music são encriptadas e precisam do módulo Widevine do Google. O Sonora usa um que transferiu do Google com o seu consentimento ou, se não houver, a cópia que um navegador daqui já tem.
settings-widevine-none = As faixas do Apple Music são encriptadas e precisam do módulo Widevine do Google. Nenhum navegador aqui tem um, por isso o Sonora pode transferi-lo do Google com o seu consentimento.
settings-widevine-looking = A procurar…
settings-widevine-asking = A aguardar a sua resposta
settings-widevine-fetching = A transferir…
settings-widevine-installing = A instalar…
settings-widevine-installed = Encontrado num navegador
settings-widevine-fetched = Transferido do Google
settings-widevine-missing = Não instalado
settings-widevine-configured = Definido pelo ambiente
settings-widevine-fetch = Transferir
settings-widevine-uninstall = Desinstalar
confirm-uninstall-widevine-title = Desinstalar o módulo Widevine?
confirm-uninstall-widevine = O Sonora elimina a cópia que transferiu do Google. As faixas do Apple Music deixam de tocar até que ele seja transferido novamente.
confirm-remove-downloads-title = Remover todas as transferências?
confirm-remove-downloads = Todas as músicas que transferiu são eliminadas deste computador, de todos os serviços. Voltam a ser transmitidas até as transferir de novo.
widevine-prompt-title = Módulo Widevine
widevine-prompt-wanted = As faixas do Apple Music são encriptadas e precisam do módulo Widevine do Google. Nenhum navegador neste computador tem um. O Sonora pode transferi-lo dos servidores do Google, o mesmo ficheiro que o Chrome instala, e guardá-lo na própria pasta.
widevine-prompt-replace = As faixas do Apple Music são encriptadas e precisam do módulo Widevine do Google. Um navegador neste computador tem um, e o Sonora pode transferir a cópia do próprio Google para usar em vez dele, o mesmo ficheiro que o Chrome instala, guardado na própria pasta.
widevine-prompt-downloading = A transferir…
widevine-prompt-terms = A versão { $version } foi transferida. Instalá-la significa aceitar os termos do Google para o módulo:
widevine-prompt-installing = A instalar…
widevine-prompt-download = Transferir
widevine-prompt-later = Agora não
widevine-prompt-accept = Aceitar e instalar
widevine-prompt-decline = Recusar
settings-equalizer = Equalizador
settings-equalizer-detail = Molda o som com uma predefinição ou uma curva sua
settings-equalizer-preset-detail = Um som pronto ou um dos seus perfis
settings-equalizer-custom = Personalizada
settings-equalizer-flat = Neutra
settings-equalizer-bass-boost = Reforço de graves
settings-equalizer-bass-reducer = Redução de graves
settings-equalizer-treble-boost = Reforço de agudos
settings-equalizer-vocal = Vocal
settings-equalizer-rock = Rock
settings-equalizer-pop = Pop
settings-equalizer-jazz = Jazz
settings-equalizer-classical = Clássica
settings-equalizer-electronic = Eletrónica
settings-equalizer-acoustic = Acústica
settings-equalizer-loudness = Loudness
equalizer-advanced = Avançado
equalizer-simple = Simples
equalizer-sound = Som
equalizer-profile-default = Predefinido
equalizer-profile-new = Perfil { $number }
equalizer-profile-pasted = Perfil colado
equalizer-profile-name-hint = Nome do perfil
equalizer-save-as = Guardar como perfil
equalizer-rename = Renomear perfil
equalizer-delete = Eliminar perfil
equalizer-delete-confirm = Eliminar “{ $name }”? Isto não pode ser anulado.
equalizer-undo = Anular
equalizer-redo = Refazer
equalizer-bass = Graves
equalizer-treble = Agudos
equalizer-tone-reset = Repor para 0 dB
equalizer-preamp = Pré-amp
equalizer-preamp-auto = Automático
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-decibels-value = { $db } dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oit
equalizer-no-gain = —
equalizer-filters = Filtros
equalizer-no-filters = Nenhum filtro ainda. Adicione um, escolha uma predefinição acima ou importe um perfil.
equalizer-add-filter = Adicionar filtro
equalizer-remove-filter = Remover filtro
equalizer-kind-peak = Pico
equalizer-kind-low-shelf = Shelf de graves
equalizer-kind-high-shelf = Shelf de agudos
equalizer-kind-low-pass = Passa-baixas
equalizer-kind-high-pass = Passa-altas
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Passa-faixa
equalizer-kind-all-pass = Passa-tudo
equalizer-transfer = Importar e exportar
equalizer-import = Importar
equalizer-export = Exportar
equalizer-import-file = Importar ficheiro…
equalizer-paste = Colar
equalizer-copy = Copiar
equalizer-export-file = Exportar ficheiro…
equalizer-imported = { $name } importado
equalizer-imported-skipped = { $name } importado sem { $count ->
        [one] uma linha
       *[other] { $count } linhas
    }
equalizer-import-nothing = Nenhuma definição de equalizador encontrada para importar
equalizer-import-too-many = Não importado: { $count ->
        [one] um filtro está ativo
       *[other] { $count } filtros estão ativos
    }, mais do que um perfil comporta
equalizer-import-too-large = Não importado: o texto ultrapassa 1 MB, demasiado grande para um perfil de equalizador
equalizer-filter-limit =Um perfil comporta até { $max } filtros.
equalizer-paste-empty = A área de transferência não tem texto
equalizer-copied = Perfil copiado como texto do Equalizer APO
equalizer-exported = Perfil exportado
equalizer-dialog-failed = Não foi possível abrir a janela de ficheiros
equalizer-file-unreadable = Não foi possível ler o ficheiro
equalizer-file-unwritable = Não foi possível gravar o ficheiro
equalizer-value-refused = O valor anterior foi mantido
equalizer-skipped-line = Linha { $line }: { $reason }
equalizer-skipped-more = e { $count ->
        [one] mais uma
       *[other] mais { $count }
    }
equalizer-skip-unknown-type = tipo de filtro desconhecido { $name }
equalizer-skip-unreadable = não é um filtro que o Sonora consegue ler
equalizer-skip-no-frequency = sem frequência
equalizer-skip-no-gain = sem ganho
equalizer-skip-no-width = sem Q ou largura de banda
equalizer-skip-expression = um valor escrito como expressão
equalizer-skip-no-preamp = o pré-amp não é um número
equalizer-skip-unsupported = { $name } não é suportado
equalizer-rules = Regras por dispositivo
equalizer-rule-other = Qualquer outro dispositivo
equalizer-rule-playing = Em reprodução
equalizer-rule-disconnected = Não ligado
equalizer-rule-replaces = Substitui a regra existente
equalizer-rule-choose-device = Escolha um dispositivo
equalizer-rule-choose-sound = Escolha um som
equalizer-rule-add = Adicionar regra
equalizer-rule-remove = Remover regra
equalizer-output-port = { $device } · { $port }
signal-path-codec-alac = ALAC
signal-path-codec-pcm = PCM
signal-path-codec-other = Outro codec
equalizer-invalid-frequency = A frequência deve estar entre { $low } e { $high } Hz
equalizer-invalid-gain = O ganho deve estar entre { $low } e { $high } dB
equalizer-invalid-q = O Q deve estar entre { $low } e { $high }
equalizer-invalid-octaves = A largura de banda deve estar entre { $low } e { $high } oitavas
equalizer-invalid-shelf-octaves = Um shelf usa Q, não largura de banda
equalizer-invalid-preamp = O pré-amp deve estar entre { $low } e { $high } dB
equalizer-overload = Clipping de { $db } dB
equalizer-overload-slight = Clipping abaixo de 0,1 dB
equalizer-graph-preamp = { $db } dB
equalizer-lower-preamp = Reduzir pré-amp
equalizer-level = Nível
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect a partir do Sonora
equalizer-status-server = Depois, o servidor de som converte para { $khz } kHz
equalizer-status-altered = Não é bit-perfect por causa de:
equalizer-status-separator = {", "}
equalizer-status-more = +{ $count }
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Normalizado
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bits
equalizer-status-float = Float → { $to } bits
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Estéreo
   *[other] { $from } canais
} → { $to ->
    [1] mono
    [2] estéreo
   *[other] { $to } canais
}
settings-lyrics = Mostrar letras
settings-lyrics-detail = Desative para ocultar as letras em todo o lado e deixar de as procurar
settings-panel-lyrics-size = Tamanho da letra (painel)
settings-panel-lyrics-size-detail = Tamanho do texto da letra no painel lateral, além do tamanho base do tipo de letra
settings-fullscreen-lyrics-size = Tamanho da letra (ecrã inteiro)
settings-fullscreen-lyrics-size-detail = Tamanho do texto da letra no leitor em ecrã inteiro, além do tamanho base do tipo de letra
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = Letras para ficheiros locais
settings-lyrics-for-local-files-detail = Usar os metadados dos ficheiros locais para procurar letras na internet
settings-artwork-for-local-files = Capas para ficheiros locais
settings-artwork-for-local-files-detail = Usar os metadados dos ficheiros locais para procurar uma capa no Deezer para o seu estado do Discord
settings-karaoke-lyrics = Letras em karaokê
settings-karaoke-lyrics-detail = Destacar letras palavra por palavra quando a sincronia estiver disponível
settings-blur-lyrics = Desfocar letras inativas
settings-blur-lyrics-detail = Desfocar as linhas seguintes e anteriores no painel de letras
settings-lyrics-sample-first = Dados arbitrários
settings-lyrics-sample-second = Insira aqui um texto engraçado
settings-lyrics-sample-japanese = ここに面白い文章を入れてください
settings-lyrics-sample-romanized = koko ni omoshiroi bunshou o irete kudasai
settings-lyrics-sample-fourth = Espero que este japonês esteja certo
settings-lyrics-sample-fifth = A quinta linha
settings-romanized-lyrics = Letras romanizadas
settings-romanized-lyrics-detail = Mostrar pronúncia gerada localmente para sistemas de escrita selecionados
settings-romanization-writing-systems = Sistemas de escrita
settings-lyrics-priority = Prioridade dos serviços
settings-lyrics-providers-detail = Escolha em que serviços procurar letras
settings-lyrics-providers-selected = { $count ->
    [one] { $count } selecionado
   *[other] { $count } selecionados
    }
settings-lyrics-provider-local = Ficheiros locais
settings-lyrics-provider-subsonic = Subsonic
settings-lyrics-provider-spotify = Spotify
settings-lyrics-provider-youtube = YouTube Music
settings-lyrics-provider-apple-music = Apple Music
settings-lyrics-provider-musixmatch = Musixmatch
settings-lyrics-provider-lrclib = LRCLIB
settings-lyrics-provider-kugou = Kugou
settings-lyrics-provider-netease = NetEase
settings-romanization-japanese = Japonês
settings-romanization-chinese = Chinês
settings-romanization-korean = Coreano
settings-romanization-cyrillic = Cirílico
settings-romanization-greek = Grego
settings-romanization-arabic = Árabe
settings-romanization-other = Outros sistemas de escrita
settings-group-window = Janela
settings-group-text = Texto
settings-group-motion = Movimento
settings-group-streaming = Streaming
settings-group-spotify-connect = Spotify Connect
settings-adaptive-menu = Menu de contexto adaptável
settings-adaptive-menu-detail = Omite entradas que a linha já mostra, como o álbum ou o artista
settings-accounts = Gerir contas
settings-accounts-detail = Os serviços que este dispositivo pode usar para reproduzir
settings-provider-current = A reproduzir deste serviço
settings-provider-sign-in = Iniciar sessão
settings-profile-account = { $provider } · { $account }
settings-sign-out = Terminar sessão
settings-lastfm = Last.fm
settings-lastfm-detail = O Sonora faz scrobbling pela sua própria conta de API do Last.fm. Crie uma e cole aqui a chave e o segredo.
settings-librefm = Libre.fm
settings-listenbrainz = ListenBrainz
settings-listenbrainz-detail = Cole um token de utilizador da sua página de definições do ListenBrainz.
settings-maloja = Maloja
settings-maloja-detail = Aponte o Sonora para o seu servidor Maloja e cole uma das chaves de API dele.
settings-scrobble-off = Não ligado
settings-scrobble-on = Ligado
settings-scrobble-waiting = A aguardar o seu navegador…
settings-scrobble-as = A fazer scrobbling como { $name }
settings-scrobble-failed = Não foi possível ligar
settings-scrobble-connect = Ligar
settings-scrobble-disconnect = Desligar
settings-scrobble-title = Ligar { $service }
settings-scrobble-request = Obter uma chave de API
settings-scrobble-token-request = Obter um token
settings-scrobble-key = Chave de API
settings-scrobble-secret = Segredo da API
settings-scrobble-token = Token de utilizador
settings-scrobble-server = Endereço do servidor
settings-local-folder = Pasta de músicas importadas
settings-local-folder-empty = Não configurada
settings-choose-folder = Escolher pasta…
settings-add-folder = Adicionar pasta
settings-remove-folder = Remover pasta
settings-rescan = Analisar novamente
settings-scan-walking = A analisar…
settings-scan-progress = { $percent }%
settings-scan-done = Concluído em { $seconds }s
settings-version = Versão
settings-version-detail = A compilação do sonora que está a executar
settings-license = Licença
settings-license-detail = GNU General Public License versão 3 ou posterior
settings-source = Código-fonte
settings-source-detail = O código-fonte correspondente a esta compilação
settings-team = Equipa
settings-role-lead-maintainer = Mantenedor principal
settings-role-maintainer = Mantenedor
settings-role-contributor = Contribuidor
settings-notice = Copyright © 2026 Contribuidores do Sonora. O Sonora é distribuído com ABSOLUTAMENTE NENHUMA GARANTIA. É software livre, e pode redistribuí-lo sob os termos da GNU General Public License versão 3 ou posterior. O Sonora não é oficial e não é afiliado ao Spotify AB.

# themes
theme-system = Sistema
theme-dark = Escuro
theme-light = Claro
theme-unavailable = { $name } (indisponível)

# corners
corners-square = Quadrado
corners-subtle = Subtil
corners-rounded = Arredondado
corners-round = Redondo

# motion
motion-system = Sistema
motion-always = Sempre
motion-never = Nunca
pace-slow = Lento
pace-base = Normal
pace-quick = Rápido
saver-off = Desligado
saver-fps = { $fps } FPS

toast-playlist-created = Playlist criada
toast-playlist-renamed = Playlist renomeada
toast-playlist-updated = Playlist atualizada
toast-playlist-cover-changed = Capa da playlist alterada
toast-playlist-cover-removed = Capa da playlist removida
toast-playlist-deleted = Playlist eliminada
toast-local-delete-failed = Alguns ficheiros de faixa não puderam ser eliminados
toast-playlist-added = Playlist adicionada à sua biblioteca
toast-playlist-removed = Playlist removida da sua biblioteca
toast-playlist-visibility = Visibilidade da playlist alterada
toast-track-added = Adicionada a { $name }
toast-track-removed = Removida de { $name }
toast-track-removed-unnamed = Removida da playlist
toast-playlist-failed = Não foi possível guardar a alteração
toast-playlist-busy = Outra alteração ainda está em curso
toast-playlist-signed-out = Inicie sessão para alterar playlists
toast-playlist-imported = Playlist { $name } importada
toast-playlist-imported-partly = Playlist { $name } importada sem { $count ->
        [one] uma música que não está
       *[other] { $count } músicas que não estão
    } na sua biblioteca
toast-playlist-import-empty = Nenhuma música de { $name } está na sua biblioteca
toast-playlist-exported = Playlist exportada
toast-file-dialog-failed = Não foi possível abrir a janela de ficheiros
toast-queued-track = { $name } adicionada à fila
toast-next-track = { $name } tocará a seguir
toast-last-track = { $name } tocará por último
toast-queued-album = Álbum adicionado à fila
toast-next-album = Álbum tocará a seguir
toast-last-album = Álbum tocará por último
toast-queued-playlist = Playlist adicionada à fila
toast-next-playlist = Playlist tocará a seguir
toast-last-playlist = Playlist tocará por último
toast-queued-artist = Artista adicionado à fila
toast-next-artist = Artista tocará a seguir
toast-last-artist = Artista tocará por último
toast-queue-failed = Não foi possível adicionar à fila
toast-queue-local-elsewhere = Ficheiros locais não podem ser colocados na fila noutro dispositivo
toast-queued-tracks = { $count } faixas adicionadas à fila
toast-next-tracks = { $count } faixas tocarão a seguir
toast-last-tracks = { $count } faixas tocarão por último
toast-queue-cleared = Fila limpa
toast-queue-removed-track = { $name } removida da fila
toast-queue-removed-tracks = { $count } faixas removidas da fila
toast-sleep-ended = Temporizador de suspensão terminado
toast-radio-failed = Não foi possível iniciar a rádio de { $name }
toast-similar-failed = Não foi possível encontrar faixas semelhantes a { $name }
toast-open-failed = Não foi possível abrir { $name }
toast-open-unsupported = { $name } não é um ficheiro de áudio que o Sonora consegue tocar
toast-open-all-failed = { $count } ficheiros não puderam ser abertos
toast-open-some-failed = Alguns ficheiros não puderam ser abertos
toast-keys-error = Erro na chave
toast-keys-refused = Spotify pode falhar
toast-why = Porquê?
toast-sign-in-to-play = { $name } só reproduz para um ouvinte com sessão iniciada
toast-restore-failed = { $name } não conseguiu voltar a ligar, por isso o Sonora continuou sem ele
toast-track-unplayable = { $name } não pôde ser reproduzida
toast-playback-stopped = A reprodução parou porque várias faixas seguidas não puderam ser tocadas
toast-widevine-failed = Falha na reprodução
toast-widevine-borrowed = Tente atualizar o Widevine
toast-open-settings = Definições
toast-throttled = { $name } está a limitar a reprodução por enquanto. Nova tentativa dentro de instantes
toast-still-throttled = { $name } ainda está a limitar a reprodução. Prima reproduzir para tentar de novo
toast-library-add-failed = { $name } não pôde ser adicionada à sua biblioteca
toast-library-remove-failed = { $name } não pôde ser removida da sua biblioteca
toast-library-added = Adicionado à sua biblioteca
toast-library-removed = Removido da sua biblioteca
toast-library-removed-one = { $name } removido da sua biblioteca
toast-library-songs-removed = { $count ->
        [one] Uma música removida
       *[other] { $count } músicas removidas
    } da sua biblioteca
toast-library-albums-removed = { $count ->
        [one] Um álbum removido
       *[other] { $count } álbuns removidos
    } da sua biblioteca
toast-playlists-removed = { $count ->
        [one] Uma playlist removida
       *[other] { $count } playlists removidas
    } da sua biblioteca
toast-favorite-removed = { $name } removido das Favoritas
toast-favorite-songs-removed = { $count ->
        [one] Uma música removida
       *[other] { $count } músicas removidas
    } das Favoritas
toast-favorite-albums-removed = { $count ->
        [one] Um álbum removido
       *[other] { $count } álbuns removidos
    } das Favoritas
toast-favorite-artists-removed = { $count ->
        [one] Um artista removido
       *[other] { $count } artistas removidos
    } das Favoritas
toast-tracks-removed = { $count ->
        [one] Uma música removida
       *[other] { $count } músicas removidas
    } de { $name }
toast-tracks-removed-unnamed = { $count ->
        [one] Uma música removida
       *[other] { $count } músicas removidas
    } da playlist
toast-unpinned = { $name } desafixado

# lyrics
lyrics-title = Letras
lyrics-idle = Toque algo para ver as letras
lyrics-loading = A procurar letras…
lyrics-missing = Nenhuma letra encontrada, lamentamos!
lyrics-instrumental = Esta música é instrumental
lyrics-failed = Não foi possível aceder ao serviço de letras
lyrics-follow = Acompanhe a música novamente
lyrics-source = Letras de { $source }
lyrics-source-local = Letras do ficheiro local
lyrics-writers = Escrita por { $writers }

update-available = Sonora { $version } está disponível
update-detail = Está na versão { $running }. Veja o que mudou ou atualize agora.
update-detail-notes = Está na versão { $running }. Veja o que mudou e atualize o Sonora da forma como o instalou.
update-notes = O que há de novo
update-now = Atualizar
update-working = A transferir a atualização…
update-failed = Não foi possível instalar a atualização. Tente novamente pela página de lançamentos.
update-never = Não verificar atualizações
notice-never = Não mostrar novamente
requirement-audio-bridge-title = O Sonora não consegue aceder ao seu servidor de som
requirement-audio-bridge = { $server ->
    [pulseaudio] O PulseAudio está em execução, mas o ALSA não tem rota até ele, por isso nada do que toca é ouvido. Instale o pulseaudio-alsa e reinicie o Sonora.
   *[pipewire] O PipeWire está em execução, mas o ALSA não tem rota até ele, por isso nada do que toca é ouvido. Instale o pipewire-alsa e reinicie o Sonora.
}
requirement-audio-output-title = Sem saída de som
requirement-audio-output = O Sonora não conseguiu abrir o seu dispositivo de áudio, por isso nada do que toca é ouvido. Verifique se há um dispositivo ligado e se outra aplicação não o está a usar.
requirement-web-engine-title = Início de sessão pelo navegador indisponível
requirement-web-engine = Instale o webkit2gtk-4.1 para iniciar sessão no YouTube Music, Deezer ou Apple Music numa janela do navegador. Até lá, cole os seus cookies.
requirement-help = O que instalar
settings-check-updates = Verificar atualizações
settings-check-updates-detail = Perguntar ao GitHub no arranque se uma nova versão está disponível. O Sonora instala a atualização sozinho apenas no Windows; noutras plataformas encaminha-o para o que mudou
settings-log = Ficheiro de log
settings-log-detail = O que o Sonora escreveu enquanto estava em execução. Anexe-o a um relatório de erro
settings-log-open = Abrir o log

# tags
tags-edit-title = Editar tags
tags-sheet-song = Música
tags-sheet-album = Álbum
tags-sheet-details = Detalhes
tags-title = Título
tags-artist = Artista
tags-artists = Artistas
tags-add-artist = Adicionar um artista
tags-remove-artist = Remover este artista
tags-track = Número da faixa
tags-track-total = Faixas no lançamento
tags-disc = Número do disco
tags-disc-total = Discos no lançamento
tags-album = Álbum
tags-album-artist = Artista do álbum
tags-year = Ano
tags-genre = Género
tags-composer = Compositor
tags-publisher = Editora
tags-isrc = ISRC
tags-comment = Comentário
toast-tags-saved = Tags guardadas para { $name }
toast-tags-failed = Não foi possível guardar as tags
toast-output-select-failed = Não foi possível selecionar { $name }
toast-output-default-failed = Não foi possível selecionar o dispositivo predefinido do sistema

nav-pin = Fixar
toast-library-pin-failed = Não foi possível atualizar o item fixado no Spotify.
toast-jam-unreadable = Esse link não leva a uma Jam em que possa entrar.
toast-jam-needs-connect = Ative “Mostrar como dispositivo Spotify” nas definições para entrar numa Jam
toast-jam-failed = Não foi possível iniciar ou entrar na Jam.
toast-jam-ended = Jam terminada
toast-jam-left = Saiu da Jam
toast-jam-leave-failed = Não foi possível terminar ou sair da Jam.
nav-nothing-pinned = Nada aqui
nav-pins-alphabetical = Alfabética
nav-pins-kind = Por tipo
nav-show-full-library = Mostrar biblioteca completa
nav-return-top = Voltar ao topo

# trouble
trouble-offline = Sem ligação
trouble-offline-detail = Verifique a sua ligação à internet e tente novamente.
trouble-not-loaded = Não foi possível carregar
trouble-retry = Tentar novamente
toast-offline = Sem ligação. Nada será transmitido até ela voltar.
toast-offline-unsupported = { $name } só toca transferências quando está online.
toast-offline-mode = Está offline. Fique online para ouvir esta música.
offline-title = Está offline.
offline-go-online = Ficar online
toast-download-failed = Não foi possível listar as faixas para transferir.
toast-percent = { $percent }%
toast-download-cancelled = Transferência de { $name } cancelada
toast-downloads-cancelled = { $count ->
    [one] Transferência de { $count } faixa cancelada
   *[other] Transferência de { $count } faixas cancelada
}
toast-settings-broken = Corrija a linha { $name } do settings.json para guardar as alterações
toast-tray-unavailable = Não há área de notificação para colocar o ícone
toast-undo = Anular
toast-link-copied = Link copiado
toast-link-sign-in = Inicie sessão no { $name } para abrir este link
toast-link-unreadable = Esse link não leva a nada que o Sonora consiga abrir
toast-code-copied = Código copiado
menu-copy-link-none = Isto não tem link para partilhar
menu-copy-link-local = Artistas e playlists locais não têm link
toast-settings-save-failed = Não foi possível guardar as definições
toast-settings-overwritten = O settings.json mudou no disco, por isso a sua última alteração não foi guardada
toast-widevine-download-failed = Não foi possível transferir o Widevine
toast-widevine-install-failed = Não foi possível instalar o Widevine
toast-widevine-remove-failed = Não foi possível remover o Widevine
toast-library-scanned = { $count ->
    [one] Análise concluída, uma música encontrada
   *[other] Análise concluída, { $count } músicas encontradas
}
toast-scan-failed = Não foi possível analisar as suas pastas de música
toast-folder-removed = Pasta { $name } removida
toast-folder-missing = { $name } já não está no disco
toast-history-removed = { $count ->
    [one] Removida do histórico
   *[other] { $count } músicas removidas do histórico
}
toast-history-remove-failed = Não foi possível remover uma música do histórico
toast-history-restore-failed = Não foi possível restaurar o histórico
toast-history-clear-failed = Não foi possível limpar o histórico

# power
wake-reason = Música em reprodução

toast-sample-plain = O evento foi criado
toast-sample-done = Guardado na sua biblioteca
toast-sample-info = Chegue 10 minutos antes do evento.
toast-sample-warning = A sua sessão termina dentro de cinco minutos
toast-sample-failed = Não foi possível aceder ao servidor
toast-sample-loading = A enviar as suas faixas
toast-sample-event = Evento criado
toast-sample-event-detail = Domingo, 3 de dezembro às 9:00
toast-sample-delete = { $name } eliminado
toast-sample-undo = Anular
toast-sample-cancel = Cancelar
toast-sample-undone = Anulado
toast-sample-queued = { $name } adicionada à fila
toast-sample-long = Este aviso diz bem mais do que os outros, por isso o texto tem de quebrar para uma segunda e talvez uma terceira linha sem empurrar o botão de fechar para fora do cartão
toast-sample-saving = A guardar { $name }
toast-sample-saved = { $name } guardado
toast-sample-save-failed = Não foi possível guardar { $name }
toast-sample-save-failed-detail = O servidor respondeu 503 Service Unavailable
toast-sample-lasting = Este fica até o fechar
toast-sample-burst = Aviso número { $name }

# Frequency response graph
response-graph-hz = { $value } Hz
response-graph-khz = { $value } kHz
response-graph-db = { $value } dB
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
