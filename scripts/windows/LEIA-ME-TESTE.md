# Dita.se no Windows: versão de teste

## Primeira instalação (uma vez só)

No PC com Windows 11, abra o menu Iniciar, digite **PowerShell**, abra e cole esta linha:

```powershell
irm https://raw.githubusercontent.com/stoll2003/ditase/windows/scripts/install.ps1 | iex
```

Aperte Enter. O script baixa a versão de teste mais recente, confere se o arquivo chegou inteiro, instala só para o seu usuário (não pede senha de administrador), cria o atalho **Dita.se** na Área de Trabalho e abre o app.

Se aparecer a tela azul "O Windows protegeu o computador", clique em **Mais informações** e depois em **Executar assim mesmo**. Ela aparece porque o Dita.se é gratuito e não tem certificado pago; o arquivo já foi conferido antes de instalar.

## Depois disso

Você não precisa copiar nem baixar mais nada. Cada vez que o Dev Founder publica uma versão de teste nova (o GitHub compila e o Mac assina e publica), o Dita.se avisa e se atualiza **quando você abrir o app**. O número da versão termina em `-teste.N`; quanto maior o N, mais nova.

Se algo der errado com a atualização, basta rodar a mesma linha de novo: ela reinstala por cima e mantém seus dados.

## Mandar o relatório de teste

No app, abra **Ajustes** e clique em **Copiar relatório de teste**. O relatório vai para a área de transferência; cole na conversa com o Dev Founder.
