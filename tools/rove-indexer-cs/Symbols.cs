using System.Collections.Concurrent;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Rove.Indexer;

static class Symbols
{
    public static ISymbol Norm(ISymbol s)
    {
        if (s is IAliasSymbol a) s = a.Target;
        if (s is IMethodSymbol m && m.ReducedFrom != null) s = m.ReducedFrom;
        return s.OriginalDefinition;
    }

    public static string SafeDisplay(ISymbol s)
    {
        try { return s.ToDisplayString(SymbolDisplayFormat.MinimallyQualifiedFormat); } catch { return s.Name; }
    }

    public static SyntaxToken? DeclIdentifier(SyntaxNode n) => n switch
    {
        BaseTypeDeclarationSyntax t => t.Identifier,
        DelegateDeclarationSyntax d => d.Identifier,
        MethodDeclarationSyntax m => m.Identifier,
        ConstructorDeclarationSyntax c => c.Identifier,
        DestructorDeclarationSyntax d => d.Identifier,
        PropertyDeclarationSyntax p => p.Identifier,
        EventDeclarationSyntax e => e.Identifier,
        IndexerDeclarationSyntax i => i.ThisKeyword,
        VariableDeclaratorSyntax v => v.Identifier,
        ParameterSyntax p when p.Identifier.Span.Length > 0 => p.Identifier,
        TypeParameterSyntax tp => tp.Identifier,
        EnumMemberDeclarationSyntax em => em.Identifier,
        SingleVariableDesignationSyntax sv => sv.Identifier,
        ForEachStatementSyntax fe => fe.Identifier,
        CatchDeclarationSyntax cd when cd.Identifier.Span.Length > 0 => cd.Identifier,
        LocalFunctionStatementSyntax lf => lf.Identifier,
        _ => null,
    };

    public static void CollectRelations(ISymbol s, ConcurrentBag<(SymbolKey, SymbolKey, RelKind)> rels, Func<ISymbol, SymbolKey> key)
    {
        switch (s)
        {
            case INamedTypeSymbol t:
                if (t.BaseType is { SpecialType: not SpecialType.System_Object and not SpecialType.System_ValueType and not SpecialType.System_Enum } bt)
                    rels.Add((key(t), key(Norm(bt)), RelKind.BaseType));
                foreach (var i in t.Interfaces) rels.Add((key(t), key(Norm(i)), RelKind.BaseType));
                foreach (var iface in t.AllInterfaces)
                    foreach (var m in iface.GetMembers())
                    {
                        if (m.IsStatic) continue;
                        var impl = t.FindImplementationForInterfaceMember(m);
                        if (impl != null && SymbolEqualityComparer.Default.Equals(impl.ContainingType, t))
                            rels.Add((key(Norm(impl)), key(Norm(m)), RelKind.Implements));
                    }
                break;
            case IMethodSymbol { OverriddenMethod: { } om } m:
                rels.Add((key(m), key(Norm(om)), RelKind.Overrides)); break;
            case IPropertySymbol { OverriddenProperty: { } op } p:
                rels.Add((key(p), key(Norm(op)), RelKind.Overrides)); break;
            case IEventSymbol { OverriddenEvent: { } oe } e:
                rels.Add((key(e), key(Norm(oe)), RelKind.Overrides)); break;
        }
    }
}
