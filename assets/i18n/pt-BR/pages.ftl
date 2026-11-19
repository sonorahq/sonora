# login
login-restoring = Verificando sua sessão salva…
login-authorizing = Aguardando autorização no seu navegador…
login-signed-in = Conectado como { $name }
login-failed-title = Falha no login
login-problem-region = O Spotify não abrirá uma sessão do país onde você está. Faça login do seu país de origem ou altere o país na sua conta do Spotify.
login-problem-credentials = Sua sessão salva do Spotify não é mais válida. Faça login novamente para continuar.
login-problem-network = O Sonora não conseguiu acessar o serviço de música. Verifique sua conexão com a internet e tente novamente.
login-problem-cancelled = Você fechou a página do navegador antes de aprovar o login. Recomece para finalizar.
login-problem-refused = O Spotify recusou o login. Aguarde um momento e tente novamente.
login-problem-premium = O Sonora usa o Spotify Premium para reproduzir, e esta conta não possui o plano. Faça login com uma conta Premium para continuar.
login-sign-in = Entrar com { $provider }
login-connect-cookies = Colar cookies manualmente
login-cookie-open = Abrir { $provider }
login-cookie-submit = Continuar
login-cookie-hint = Cole o cabeçalho Cookie aqui
login-cookie-step-1 = Acesse music.youtube.com e certifique-se de estar conectado. Funciona melhor em uma janela anônima.
login-cookie-step-2 = Pressione F12, abra a guia Rede e recarregue a página.
login-cookie-step-3 = Selecione qualquer requisição chamada "browse" ou "next".
login-cookie-step-4 = Em Cabeçalhos, encontre Cookie em Cabeçalhos da Requisição, clique com o botão direito e copie o valor.
login-cookie-step-note = Cole o valor completo abaixo: o painel Cookies da requisição não é suficiente, pois o valor precisa conter SAPISID e __Secure-3PAPISID.
login-cookie-header-title = Cole seus cookies do { $provider } para concluir o login
login-cookie-apple-step-3 = Selecione qualquer requisição para { $site }.
login-cookie-apple-note = Cole o valor completo, incluindo { $cookie }.
login-cookie-named-title = Cole seu cookie do { $provider } para concluir o login
login-cookie-named-hint = Cole o valor do cookie aqui
login-cookie-named-step-1 = Acesse { $site } e certifique-se de estar conectado.
login-cookie-named-step-2 = Pressione F12 e abra a guia Armazenamento no Firefox ou a guia Aplicativo no Chrome.
login-cookie-named-step-3 = Expanda Cookies, selecione { $site } e encontre o cookie chamado { $cookie }.
login-cookie-named-step-4 = Clique duas vezes no valor e copie-o.
login-cookie-named-note = Só o valor de { $cookie } já basta. Um cabeçalho Cookie inteiro copiado da guia Rede também funciona.
login-guest-title = Modo convidado
login-guest-use = Usar modo convidado
login-usage-consent = Ajude-nos a estimar quantas pessoas usam o Sonora.
login-device-code = Digite este código em { $url }
login-server-title = Conecte-se ao seu servidor Subsonic
login-server-detail = Digite o endereço de qualquer servidor Subsonic ou OpenSubsonic (Navidrome, Airsonic, Gonic, …) e faça login com seu usuário e senha do servidor. A sessão permanece neste dispositivo.
login-server-hint = https://music.example.com
login-username-hint = Usuário
login-password-hint = Senha
login-server-submit = Conectar
login-account-title = Escolha uma conta
login-account-detail = Esta sessão está conectada a mais de uma conta Google. Escolha qual o Sonora deve usar.
login-choose-title = Entrar no { $provider }
login-choose-detail = Escolha como entrar no { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artista
artist-monthly-listeners = { $count ->
    [one] { $value } ouvinte mensal
   *[other] { $value } ouvintes mensais
}
artist-popular = Populares
artist-popular-eyebrow = Explorar este artista
artist-popular-empty = Nada para tocar deste artista ainda
artist-releases = Lançamentos
artist-filter-all = Tudo
artist-filter-singles = Singles
artist-filter-eps = EPs
artist-appears-on = Aparece em
album-also-like = Você também pode gostar
album-tab-artists = Artistas
album-label = ℗ { $label }

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

# home page
home-quick-picks = Escolhas rápidas
home-quick-picks-empty = Curta algumas músicas e elas aparecerão aqui
home-recently-added = Adicionados recentemente
home-recently-played = Tocados recentemente
home-favorite-albums = Álbuns favoritos
home-artists = Artistas
home-collection-albums = Álbuns da sua coleção

# search page
search-placeholder = O que você quer ouvir?
search-browse = Explorar tudo
search-submit = Buscar
search-recent = Buscas recentes
search-forget = Remover do histórico
genre-empty = Nada para mostrar aqui ainda
search-best-match = Melhor resultado
search-no-matches = Nenhum resultado
search-results = Resultados
search-artists = Artistas
search-albums-playlists = Álbuns e playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } música na Biblioteca
       *[other] { $count } músicas na Biblioteca
    }

# song page
song-eyebrow = Música
song-view-album = Ver álbum
song-about = Sobre esta música
song-album = Álbum
song-released = Lançada
song-streams = Reproduções
song-position = Posição
song-label = Gravadora
song-popularity = Popularidade
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
tags-genre = Gênero
tags-composer = Compositor
tags-publisher = Editora
tags-isrc = ISRC
tags-comment = Comentário
