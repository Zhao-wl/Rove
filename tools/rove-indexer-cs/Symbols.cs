using System.Collections.Concurrent;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
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
        // #212：运算符声明的记号就是运算符本身，口径与使用处一致（`public static V3 operator +(…)` 记在 `+` 上）。
        OperatorDeclarationSyntax o => o.OperatorToken,
        // 转换运算符没有运算符记号；用 `operator` 关键字，避免和目标类型名自己的类型引用出现位置重叠。
        ConversionOperatorDeclarationSyntax c => c.OperatorKeyword,
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

    /// 使用处的运算符 / 索引器记号（#212）。标识符之外，这些位置也要产生出现位置，
    /// 否则 `a + b` 的 `+`、`x[i]` 的 `[` 既跳不到声明、也查不到引用。
    ///
    /// 取记号本身的位置：`tests/nav-cases` 里运算符用例的查询点写的就是运算符，索引器写的是 `[`。
    /// 这里只做语法层的筛选（能排掉 `is` / `as` / `??` 这些绝不会有用户定义符号的形态），
    /// 是不是"用户定义"由 <see cref="IsUserDefinedOperatorOrIndexer"/> 在绑定后判定。
    public static SyntaxToken? UseToken(SyntaxNode n) => n switch
    {
        BinaryExpressionSyntax b when !b.OperatorToken.IsKind(SyntaxKind.IsKeyword)
            && !b.OperatorToken.IsKind(SyntaxKind.AsKeyword)
            && !b.OperatorToken.IsKind(SyntaxKind.QuestionQuestionToken) => b.OperatorToken,
        PrefixUnaryExpressionSyntax p => p.OperatorToken,
        PostfixUnaryExpressionSyntax p => p.OperatorToken,
        // 复合赋值 `a += b` 绑定到 `operator +`；简单赋值与 `??=` 没有用户定义的运算符。
        AssignmentExpressionSyntax a when !a.OperatorToken.IsKind(SyntaxKind.EqualsToken)
            && !a.OperatorToken.IsKind(SyntaxKind.QuestionQuestionEqualsToken) => a.OperatorToken,
        ElementAccessExpressionSyntax e => e.ArgumentList.OpenBracketToken,
        // `a?[i]`
        ElementBindingExpressionSyntax e => e.ArgumentList.OpenBracketToken,
        // 对象初始化器里的 `new D { [k] = v }`
        ImplicitElementAccessSyntax i => i.ArgumentList.OpenBracketToken,
        _ => null,
    };

    /// 只记用户定义的运算符与索引器：内建运算符（`int + int`）与数组下标没有可跳转的声明，
    /// 记下来只会让出现位置列表白白变大。
    public static bool IsUserDefinedOperatorOrIndexer(ISymbol? s) => s switch
    {
        IMethodSymbol m => m.MethodKind is MethodKind.UserDefinedOperator or MethodKind.Conversion,
        IPropertySymbol p => p.IsIndexer,
        _ => false,
    };

    /// The source-level type to which Go to Type Definition should navigate.
    public static ITypeSymbol? DeclaredType(ISymbol symbol)
    {
        ITypeSymbol? type = symbol switch
        {
            ILocalSymbol local => local.Type,
            IFieldSymbol field => field.Type,
            IPropertySymbol property => property.Type,
            IParameterSymbol parameter => parameter.Type,
            IEventSymbol evt => evt.Type,
            IMethodSymbol { ReturnsVoid: false } method => method.ReturnType,
            _ => null,
        };
        // These forms do not have declarations of their own.
        while (true)
        {
            if (type is IArrayTypeSymbol array) type = array.ElementType;
            else if (type is IPointerTypeSymbol pointer) type = pointer.PointedAtType;
            else if (type is INamedTypeSymbol { OriginalDefinition.SpecialType: SpecialType.System_Nullable_T } nullable)
                type = nullable.TypeArguments[0];
            else break;
        }
        return type switch
        {
            INamedTypeSymbol
            {
                TypeKind: not TypeKind.Error,
                SpecialType: SpecialType.None,
                IsAnonymousType: false,
                IsTupleType: false,
            } named => named.OriginalDefinition,
            ITypeParameterSymbol parameter => parameter.OriginalDefinition,
            _ => null,
        };
    }

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
