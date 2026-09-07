# common
common-on = Ligado
common-off = Desligado
common-left = Esquerda
common-right = Direita
common-search = Pesquisar
common-unknown = Desconhecido
common-not-provided = Não informado
common-not-available = Indisponível
common-cancel = Cancelar
common-save = Salvar
common-delete = Excluir
common-play = Tocar
common-more = Mais
common-previous = Anterior
common-next = Próximo
common-dismiss = Dispensar
common-clear = Limpar
number-group = { "." }

# navigation
nav-history = Histórico
nav-home = Início
nav-search = Buscar
nav-library = Sua Biblioteca
nav-settings = Configurações
nav-songs = Músicas
nav-favorites = Favoritas
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
library-play-liked-songs = Tocar
library-no-songs = Nenhuma favorita ainda
library-no-albums = Nenhum álbum salvo ainda
library-no-playlists = Nenhuma playlist ainda
library-no-artists = Nenhum artista seguido ainda
library-no-local-songs = Nenhuma música importada encontrada
library-no-local-favorites = Nenhuma favorita local ainda
library-no-local-albums = Nenhum álbum importado encontrado
library-no-local-artists = Nenhum artista importado encontrado
library-no-local-playlists = Nenhuma playlist local ainda
library-no-matches = Nenhum resultado
library-not-loaded = Sua biblioteca não carregou
library-part-not-loaded = Esta parte da sua biblioteca não carregou
library-local-unconfigured = Configure sua biblioteca local

# app menu
app-refresh-library = Atualizar Biblioteca
app-sign-out = Sair
app-quit = Sair do aplicativo

# table columns
column-played-at = Tocada em
column-index = #
column-title = Título
column-artist = Artista
column-album = Álbum
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
menu-add-to-playlist = Adicionar à playlist
menu-add-tracks-to-playlist = { $count ->
    [one] Adicionar { $count } faixa à playlist
   *[other] Adicionar { $count } faixas à playlist
}
menu-new-playlist = Nova playlist
menu-edit-tags = Editar tags
menu-no-playlists = Nenhuma playlist
menu-add-to-library = Adicionar às Favoritas
menu-add-tracks-to-library = { $count ->
    [one] Adicionar { $count } faixa às Favoritas
   *[other] Adicionar { $count } faixas às Favoritas
}
menu-remove-from-library = Remover das Favoritas
menu-remove-tracks-from-library = { $count ->
    [one] Remover { $count } faixa das Favoritas
   *[other] Remover { $count } faixas das Favoritas
}
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
menu-song-radio = Ir para a rádio da música
menu-go-to-album = Ir para o álbum
menu-go-to-artist = Ir para o artista
menu-view-details = Ver detalhes
menu-copy-link = Copiar link
menu-cut = Recortar
menu-copy = Copiar
menu-paste = Colar
menu-select-all = Selecionar tudo
menu-remove-from-queue = Remover da fila
menu-open-playlist = Abrir playlist
menu-play-playlist = Tocar playlist
menu-rename-playlist = Renomear playlist
menu-delete-playlist = Excluir playlist
menu-add-playlist-to-library = Adicionar à Biblioteca
menu-remove-playlist-from-library = Remover da Biblioteca
menu-make-playlist-public = Tornar pública
menu-make-playlist-private = Tornar privada
menu-open-album = Abrir álbum
menu-play-album = Tocar álbum
menu-play-artist = Tocar artista

# playlist editor
playlist-name-placeholder = Nome da playlist
playlist-create-title = Criar playlist
playlist-rename-title = Renomear playlist
playlist-delete-title = Excluir playlist
playlist-delete-confirm = Excluir "{ $name }"? Isto não pode ser desfeito.
playlist-again-title = Adicionar novamente?
playlist-again-confirm = Esta faixa já está em "{ $name }". Adicionar outra cópia?
playlist-again-add = Adicionar novamente

# confirm
confirm-remove-library-title = Remover da biblioteca
confirm-remove-playlist-title = Remover da playlist
confirm-remove-history-title = Remover do histórico
confirm-unfollow-title = Deixar de seguir
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
confirm-unfollow-artists = { $count ->
    [one] Deixar de seguir este artista?
   *[other] Deixar de seguir { $count } artistas?
}
confirm-remove-playlists = { $count ->
    [one] Remover esta playlist da sua biblioteca?
   *[other] Remover { $count } playlists da sua biblioteca?
}

# queue panel
queue-title = Fila
queue-history = Histórico
queue-now-playing = Tocando agora
queue-from = De
queue-up-next = A seguir
queue-reset = Redefinir
queue-clear = Limpar
queue-empty = Sua fila está vazia
queue-similar = Faixas semelhantes
queue-radio = Reprodução automática de faixas semelhantes

# player bar
player-nothing-playing = Nada tocando
player-percent = { $value }%
player-shuffle = Aleatório
player-repeat = Repetir
player-repeat-all = Repetir tudo
player-repeat-one = Repetir uma
player-mute = Silenciar
player-unmute = Ativar som
player-previous = Faixa anterior
player-next = Próxima faixa
player-fullscreen = Tela cheia
player-fullscreen-leave = Sair da tela cheia
fullscreen-artwork = Capa

# filters
filter-history = Filtrar histórico de reprodução
history-empty = As faixas que você tocar aparecerão aqui.
history-not-loaded = O histórico de reprodução não pôde ser carregado.
history-clear = Limpar histórico
history-clear-title = Limpar histórico de reprodução
history-clear-confirm = Todas as reproduções serão removidas deste dispositivo. Isto não pode ser desfeito.
filter-library = Filtrar sua biblioteca
filter-album = Filtrar faixas do álbum
filter-reset = Redefinir filtros
filter-duration = Duração
filter-year = Ano
filter-explicit = Explícitas apenas
filter-playable = Reproduzíveis apenas

# view
view-list = Lista
view-cards = Grade

# toolbar
tool-columns = Colunas
tool-sort = Ordenar
tool-filters = Filtros

# login
login-signed-out = Faça login para carregar sua biblioteca musical
login-restoring = Verificando sua sessão salva…
login-authorizing = Aguardando autorização no seu navegador…
login-signed-in = Conectado como { $name }
login-failed-title = Falha no login
login-problem-region = O Spotify não abrirá uma sessão do país onde você está. Faça login do seu país de origem ou altere o país na sua conta do Spotify.
login-problem-credentials = Sua sessão salva do Spotify não é mais válida. Faça login novamente para continuar.
login-problem-network = O Sonora não conseguiu acessar o Spotify. Verifique sua conexão com a internet e tente novamente.
login-problem-cancelled = Você fechou a página do navegador antes de aprovar o login. Recomece para finalizar.
login-problem-refused = O Spotify recusou o login. Aguarde um momento e tente novamente.
login-problem-premium = O Sonora usa o Spotify Premium para reproduzir, e esta conta não possui o plano. Faça login com uma conta Premium para continuar.
login-sign-in = Entrar com { $provider }
login-connect-cookies = Colar cookies manualmente
login-use = Usar { $provider }
login-guest-title = Modo convidado
login-guest-use = Usar modo convidado
login-guest-detail = Navegue e toque sem uma conta. Sua biblioteca, curtidas e playlists ficam inacessíveis.
login-usage-consent = Ajude-nos a estimar quantas pessoas usam o Sonora.
login-device-code = Digite este código em { $url }
login-cookie-submit = Continuar
login-cookie-hint = Cole o cabeçalho Cookie aqui
login-cookie-step-1 = Acesse music.youtube.com e certifique-se de estar conectado.
login-cookie-step-2 = Pressione F12, abra a guia Rede e recarregue a página.
login-cookie-step-3 = Selecione qualquer requisição chamada "browse" ou "next".
login-cookie-step-4 = Em Cabeçalhos, encontre Cookie em Cabeçalhos da Requisição, clique com o botão direito e copie o valor.
login-cookie-step-note = Cole o valor completo abaixo: o painel Cookies da requisição não é suficiente, pois o valor precisa conter SAPISID e __Secure-3PAPISID.
login-cookie-title = Cole seus cookies do YouTube Music para finalizar o login
login-account-title = Escolha uma conta
login-account-detail = Esta sessão está conectada a mais de uma conta Google. Escolha qual o Sonora deve usar.

# album and playlist pages
detail-album = Álbum
detail-playlist = Playlist
detail-play-album = Tocar álbum
detail-play-playlist = Tocar playlist

# play button
play-pause = Pausar
play-resume = Retomar
play-loading = Carregando…

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } ouvinte mensal
   *[other] { $value } ouvintes mensais
}
artist-play = Tocar agora
artist-follow = Seguir
artist-unfollow = Deixar de seguir
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

# user profile page
user-eyebrow = Perfil
user-followers = { $count ->
    [one] { $value } seguidor
   *[other] { $value } seguidores
}
user-following = { $count ->
   *[other] { $value } seguindo
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
home-listen-again = Ouvir novamente
home-quick-picks-eyebrow = Comece por uma música
home-quick-picks-empty = Curta algumas músicas e elas aparecerão aqui

# search page
search-placeholder = O que você quer ouvir?
search-browse = Explorar tudo
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
song-play = Tocar música
song-view-album = Ver álbum
song-loading = Carregando informações da música…
song-about = Sobre esta música
song-album = Álbum
song-released = Lançada
song-streams = Reproduções
song-position = Posição
song-label = Gravadora
song-popularity = Popularidade
song-popularity-value = { $value }%
song-disc-track = Disco { $disc }, faixa { $track }
song-track = Faixa { $track }
song-credits = Créditos
song-performed-by = Interpretada por
song-details = Gêneros e detalhes
song-genres = Gêneros
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
settings-tab-general = Geral
settings-tab-appearance = Aparência
settings-tab-playback = Reprodução
settings-theme = Tema
settings-theme-detail = Escolha a paleta de cores do aplicativo
settings-opacity = Opacidade
settings-opacity-detail = Ajuste a opacidade do fundo do aplicativo
settings-opacity-value = { $percent }%
settings-theme-config = Abrir configuração
settings-adaptive = Tema adaptável
settings-adaptive-detail = Tingir a paleta com a capa do álbum em reprodução
settings-icons = Pacote de ícones
settings-icons-detail = Escolha o conjunto de ícones usado na interface
settings-motion = Reduzir movimento
settings-motion-detail = Pular animações e transições da interface
settings-pace = Velocidade da animação
settings-pace-detail = A velocidade das animações da interface
settings-saver = Economia de bateria
settings-saver-detail = Limitar a taxa de quadros das animações quando o Sonora não está em foco, aplicado a partir da próxima inicialização
settings-corners = Cantos
settings-corners-detail = O quão arredondados são as superfícies e os controles
settings-font = Tamanho da fonte
settings-font-detail = Tamanho base do texto, tudo o mais escala junto
settings-font-value = { $size } px
settings-startup = Mostrar na inicialização
settings-startup-detail = A tela que o Sonora abre ao iniciar
settings-entries = Itens da barra lateral
settings-entries-detail = As seções listadas na barra lateral
settings-entries-pick = Escolher itens
settings-language = Idioma
settings-language-detail = O idioma usado pela interface do Sonora
settings-language-system = Sistema
settings-language-search = Pesquisar idioma
settings-language-none = Nenhum idioma encontrado
settings-typeface = Fonte
settings-typeface-detail = A fonte usada pelo Sonora na interface
settings-typeface-system = Padrão
settings-typeface-search = Pesquisar fonte
settings-typeface-none = Nenhuma fonte encontrada
settings-window-controls = Botões da janela
settings-window-controls-detail = Desenhar minimizar, maximizar e fechar na barra de título
settings-controls-side = Lado dos botões
settings-controls-side-detail = Em qual extremidade da barra de título os botões ficam
settings-normalisation = Normalizar volume
settings-normalisation-detail = Mantém as faixas em um volume consistente
settings-gapless = Reprodução contínua
settings-gapless-detail = Toca uma faixa após a outra sem pausa, como um álbum foi sequenciado
settings-karaoke-lyrics = Letras em karaokê
settings-karaoke-lyrics-detail = Destacar letras palavra por palavra quando a sincronia estiver disponível
settings-romanized-lyrics = Letras romanizadas
settings-romanized-lyrics-detail = Mostrar pronúncia gerada localmente para sistemas de escrita selecionados
settings-romanization-writing-systems = Sistemas de escrita
settings-romanization-japanese = Japonês
settings-romanization-chinese = Chinês
settings-romanization-korean = Coreano
settings-romanization-cyrillic = Cirílico
settings-romanization-greek = Grego
settings-romanization-arabic = Árabe
settings-romanization-other = Outros sistemas de escrita
settings-advanced = Avançado
settings-group-accounts = Contas
settings-group-library = Biblioteca
settings-group-text = Texto
settings-group-motion = Movimento
settings-group-title-bar = Barra de título
settings-group-lyrics = Letras
settings-group-project = Projeto
settings-adaptive-menu = Menu de contexto adaptável
settings-adaptive-menu-detail = Omite entradas que a linha já mostra, como o álbum ou o artista
settings-accounts = Gerenciar contas
settings-accounts-detail = Os serviços que este dispositivo pode usar para reproduzir
settings-provider-none = Não conectado
settings-provider-connected = Conectado
settings-provider-current = Reproduzindo deste serviço
settings-provider-guest = Reproduzindo como convidado
settings-provider-switch = Mudar para
settings-sign-out = Sair
settings-local-folder = Pasta de músicas importadas
settings-local-folder-empty = Não configurada
settings-choose-folder = Escolher pasta…
settings-rescan = Revarrer
settings-clear-folder = Limpar
settings-tab-about = Sobre
settings-version = Versão
settings-version-detail = A compilação do sonora que você está executando
settings-license = Licença
settings-license-detail = GNU General Public License versão 3 ou posterior
settings-license-view = Ler a licença
settings-source = Código-fonte
settings-source-detail = O código-fonte correspondente a esta compilação
settings-source-view = Abrir o repositório
settings-team = Equipe
settings-team-github = GitHub
settings-role-lead-maintainer = Mantenedor Principal
settings-role-maintainer = Mantenedor
settings-role-contributor = Contribuidor
settings-notice = Copyright © 2026 Contribuidores do Sonora. O Sonora é distribuído com ABSOLUTAMENTE NENHUMA GARANTIA. É um software livre, e você pode redistribuí-lo sob os termos da GNU General Public License versão 3 ou posterior. O Sonora não é oficial e não é afiliado ao Spotify AB.

# themes
theme-system = Sistema
theme-dark = Escuro
theme-light = Claro
theme-midnight = Meia-noite
theme-forest = Floresta
theme-ocean = Oceano
theme-rose = Rosa
theme-lavender = Lavanda
theme-amber = Âmbar

# corners
corners-square = Quadrado
corners-subtle = Sutil
corners-rounded = Arredondado
corners-round = Redondo

# motion
motion-system = Sistema
motion-always = Sempre
motion-never = Nunca
pace-slow = Lento
pace-base = Padrão
pace-quick = Rápido
saver-off = Desligado
saver-light = Leve ({ $fps } FPS)
saver-medium = Médio ({ $fps } FPS)
saver-strong = Intenso ({ $fps } FPS)

toast-playlist-created = Playlist criada
toast-playlist-renamed = Playlist renomeada
toast-playlist-deleted = Playlist excluída
toast-playlist-added = Playlist adicionada à sua biblioteca
toast-playlist-removed = Playlist removida da sua biblioteca
toast-playlist-visibility = Visibilidade da playlist alterada
toast-track-added = Adicionada a { $name }
toast-track-removed = Removida de { $name }
toast-playlist-failed = Não foi possível salvar a alteração
toast-playlist-busy = Outra alteração ainda está em andamento
toast-playlist-signed-out = Faça login para alterar playlists
toast-queued-track = { $name } adicionada à fila
toast-next-track = { $name } tocará a seguir
toast-queued-album = Álbum adicionado à fila
toast-next-album = Álbum tocará a seguir
toast-queued-playlist = Playlist adicionada à fila
toast-next-playlist = Playlist tocará a seguir
toast-queued-artist = Artista adicionado à fila
toast-next-artist = Artista tocará a seguir
toast-queue-failed = Não foi possível adicionar à fila
toast-keys-refused = O Spotify não está concedendo chaves de reprodução para esta conta
toast-sign-in-to-play = { $name } só reproduz para um ouvinte conectado
toast-track-unplayable = { $name } não pôde ser reproduzida
toast-library-add-failed = { $name } não pôde ser adicionada à sua biblioteca
toast-library-remove-failed = { $name } não pôde ser removida da sua biblioteca

# lyrics
lyrics-title = Letras
lyrics-idle = Toque algo para ver suas letras
lyrics-loading = Procurando letras…
lyrics-missing = Nenhuma letra encontrada, desculpe!
lyrics-instrumental = Esta música é instrumental
lyrics-failed = Não foi possível acessar o serviço de letras
lyrics-follow = Acompanhe a música novamente
lyrics-source = Letras de { $source }
lyrics-writers = Escrita por { $writers }

update-available = Sonora { $version } está disponível
update-detail = Você está na versão { $running }. Veja o que mudou ou atualize agora.
update-detail-notes = Você está na versão { $running }. Veja o que mudou e atualize o Sonora da forma como você o instalou.
update-notes = O que há de novo
update-now = Atualizar
update-later = Depois
update-working = Baixando a atualização…
update-failed = Não foi possível instalar a atualização. Tente novamente pela página de lançamentos.
settings-check-updates = Verificar atualizações
settings-check-updates-detail = Perguntar ao GitHub na inicialização se uma nova versão está disponível. O Sonora instala a atualização sozinho apenas no Windows; em outras plataformas ele te direciona ao que mudou

# tags
tags-edit-title = Editar tags
tags-sheet-song = Música
tags-sheet-album = Álbum
tags-sheet-details = Detalhes
tags-title = Título
tags-artist = Artista
tags-track = Número da faixa
tags-track-total = Faixas no lançamento
tags-disc = Número do disco
tags-disc-total = Discos no lançamento
tags-album = Álbum
tags-album-artist = Artista do álbum
tags-year = Ano
tags-genre = Gênero
tags-composer = Compositor
tags-publisher = Editora
tags-isrc = ISRC
tags-comment = Comentário
toast-tags-saved = Tags salvas para { $name }
toast-tags-failed = Não foi possível salvar as tags
